//! Vector stores for the pipeline tests: an in-memory fake that implements the
//! Qdrant REST subset Tails uses, or a real Qdrant (`QDRANT_TEST_ENDPOINT`).
//!
//! The fake stores the JSON the writer actually sent and evaluates the reader's
//! filters and hybrid queries itself, so a key name, vector name, value or format that
//! only one side changed makes a test fail. Queries follow Qdrant 1.19 (checked
//! against a real server in [`tests::query_api_matches_real_qdrant`]): named cosine
//! dense vectors, sparse vectors scored as `Σ query value · idf · stored value` over
//! shared indices with Qdrant's IDF modifier, and prefetches fused by reciprocal rank
//! fusion. Anything it does not implement (an endpoint, a filter condition, a query
//! form, a collection option) is recorded and fails [`Store::finish`] rather than
//! silently matching.

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use rag_core::qdrant::Qdrant;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

#[derive(Clone, Default)]
pub struct FakeQdrant {
    collections: Arc<Mutex<BTreeMap<String, Collection>>>,
    unsupported: Arc<Mutex<Vec<String>>>,
}

fn ok(result: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({"result": result, "status": "ok", "time": 0.0}))
}

fn bad_request(msg: String) -> ResponseTemplate {
    ResponseTemplate::new(400).set_body_json(json!({"status": {"error": msg}}))
}

/// Qdrant point IDs are unsigned integers or UUIDs (returned lowercase, hyphenated).
fn point_id(v: &Value) -> Result<String, String> {
    match v {
        Value::Number(n) if n.is_u64() => Ok(n.to_string()),
        Value::String(s) => uuid::Uuid::parse_str(s)
            .map(|u| u.to_string())
            .map_err(|_| format!("invalid point id {s:?}")),
        other => Err(format!("invalid point id {other}")),
    }
}

fn id_value(id: &str) -> Value {
    id.parse::<u64>()
        .map(Value::from)
        .unwrap_or_else(|_| Value::String(id.to_string()))
}

/// Payload value at a dotted key such as `Metadata.chunk_of`.
fn lookup<'a>(payload: &'a Value, key: &str) -> Option<&'a Value> {
    key.split('.').try_fold(payload, |v, part| v.get(part))
}

/// Datetime formats Qdrant accepts for datetime payloads and range bounds.
fn parse_datetime(s: &str) -> Option<DateTime<Utc>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    for fmt in [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M",
    ] {
        if let Ok(dt) = NaiveDateTime::parse_from_str(s, fmt) {
            return Some(dt.and_utc());
        }
    }
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .ok()
        .map(|d| d.and_hms_opt(0, 0, 0).unwrap().and_utc())
}

fn in_range(range: &Value, value: &Value) -> Result<bool, String> {
    let bounds = range.as_object().ok_or("range must be an object")?;
    let datetime = bounds.values().any(Value::is_string);
    for (op, bound) in bounds {
        let ord = if datetime {
            // A payload value that is not a datetime never matches a datetime range.
            let (Some(v), Some(b)) = (
                value.as_str().and_then(parse_datetime),
                bound.as_str().and_then(parse_datetime),
            ) else {
                return Ok(false);
            };
            v.cmp(&b)
        } else {
            let (Some(v), Some(b)) = (value.as_f64(), bound.as_f64()) else {
                return Ok(false);
            };
            v.total_cmp(&b)
        };
        let pass = match op.as_str() {
            "gte" => ord.is_ge(),
            "gt" => ord.is_gt(),
            "lte" => ord.is_le(),
            "lt" => ord.is_lt(),
            other => return Err(format!("unsupported range operator {other}")),
        };
        if !pass {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Qdrant `match`: the stored value, or any element of a stored array, equals.
fn matches_value(stored: Option<&Value>, expected: &Value) -> bool {
    match stored {
        Some(Value::Array(items)) => items.contains(expected),
        Some(v) => v == expected,
        None => false,
    }
}

fn condition(cond: &Value, id: &str, payload: &Value) -> Result<bool, String> {
    if cond.get("must").is_some() || cond.get("should").is_some() || cond.get("must_not").is_some()
    {
        return filter(cond, id, payload);
    }
    if let Some(ids) = cond.get("has_id") {
        let ids = ids.as_array().ok_or("has_id must be an array")?;
        return Ok(ids.iter().any(|v| point_id(v).is_ok_and(|v| v == id)));
    }
    if let Some(inner) = cond.get("is_empty") {
        let key = inner["key"].as_str().ok_or("is_empty needs a key")?;
        return Ok(match lookup(payload, key) {
            None | Some(Value::Null) => true,
            Some(Value::Array(a)) => a.is_empty(),
            Some(_) => false,
        });
    }
    if let Some(inner) = cond.get("is_null") {
        let key = inner["key"].as_str().ok_or("is_null needs a key")?;
        return Ok(matches!(lookup(payload, key), Some(Value::Null)));
    }
    let key = cond["key"]
        .as_str()
        .ok_or_else(|| format!("unsupported condition {cond}"))?;
    let stored = lookup(payload, key);
    if let Some(m) = cond.get("match") {
        if let Some(v) = m.get("value") {
            return Ok(matches_value(stored, v));
        }
        if let Some(any) = m.get("any").and_then(Value::as_array) {
            return Ok(any.iter().any(|v| matches_value(stored, v)));
        }
        if let Some(except) = m.get("except").and_then(Value::as_array) {
            return Ok(!except.iter().any(|v| matches_value(stored, v)));
        }
        return Err(format!("unsupported match {m}"));
    }
    if let Some(range) = cond.get("range") {
        return match stored {
            Some(v) => in_range(range, v),
            None => Ok(false),
        };
    }
    Err(format!("unsupported condition {cond}"))
}

/// Qdrant filter semantics: every `must`, at least one `should` (when given), no `must_not`.
fn filter(f: &Value, id: &str, payload: &Value) -> Result<bool, String> {
    let list = |k: &str| -> Result<Vec<Value>, String> {
        match f.get(k) {
            None | Some(Value::Null) => Ok(vec![]),
            Some(Value::Array(a)) => Ok(a.clone()),
            Some(single @ Value::Object(_)) => Ok(vec![single.clone()]),
            Some(other) => Err(format!("{k} must be a list: {other}")),
        }
    };
    for c in list("must")? {
        if !condition(&c, id, payload)? {
            return Ok(false);
        }
    }
    let should = list("should")?;
    if !should.is_empty() {
        let mut any = false;
        for c in should {
            any |= condition(&c, id, payload)?;
        }
        if !any {
            return Ok(false);
        }
    }
    for c in list("must_not")? {
        if condition(&c, id, payload)? {
            return Ok(false);
        }
    }
    for k in f.as_object().map(|o| o.keys()).into_iter().flatten() {
        if !["must", "should", "must_not"].contains(&k.as_str()) {
            return Err(format!("unsupported filter clause {k}"));
        }
    }
    Ok(true)
}

/// Qdrant stores cosine vectors normalized, so cosine similarity is a dot product.
fn normalized(mut v: Vec<f32>) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        v.iter_mut().for_each(|x| *x /= norm);
    }
    v
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn numbers(v: &Value, what: &str) -> Result<Vec<f32>, String> {
    v.as_array()
        .ok_or_else(|| format!("{what} must be an array"))?
        .iter()
        .map(|x| {
            x.as_f64()
                .map(|x| x as f32)
                .ok_or_else(|| format!("{what} must be numbers"))
        })
        .collect()
}

/// A sparse vector `{"indices": [...], "values": [...]}`: unique u32 indices.
fn sparse(v: &Value) -> Result<Vec<(u32, f32)>, String> {
    let obj = v.as_object().ok_or("sparse vector must be an object")?;
    if let Some(k) = obj
        .keys()
        .find(|k| !["indices", "values"].contains(&k.as_str()))
    {
        return Err(format!("unsupported sparse vector key {k}"));
    }
    let indices: Vec<u32> = v["indices"]
        .as_array()
        .ok_or("sparse vector needs indices")?
        .iter()
        .map(|i| {
            i.as_u64()
                .and_then(|i| u32::try_from(i).ok())
                .ok_or_else(|| format!("sparse index {i} is not a u32"))
        })
        .collect::<Result<_, _>>()?;
    let values = numbers(&v["values"], "sparse values")?;
    if indices.len() != values.len() {
        return Err("sparse indices and values differ in length".into());
    }
    let mut seen = std::collections::BTreeSet::new();
    if !indices.iter().all(|i| seen.insert(*i)) {
        return Err("duplicate sparse index".into());
    }
    Ok(indices.into_iter().zip(values).collect())
}

/// Sort key of a point ID: numbers (numerically) before UUIDs.
fn id_key(id: &str) -> (u8, u64, &str) {
    match id.parse::<u64>() {
        Ok(n) => (0, n, id),
        Err(_) => (1, 0, id),
    }
}

/// Scores descending, ties by point ID. Qdrant leaves the order of equal scores
/// unspecified (it varies between identical requests), so tests must not depend on it.
fn sort_scored(scored: &mut [(f32, String)]) {
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(id_key(&a.1).cmp(&id_key(&b.1))));
}

/// Qdrant's reciprocal rank fusion: a point at 0-based position `r` of a list scores
/// `1 / (k + r)`, summed over the lists it appears in.
fn rrf(lists: &[Vec<String>], k: f32) -> Vec<(f32, String)> {
    let mut scores: BTreeMap<&str, f32> = BTreeMap::new();
    for list in lists {
        for (rank, id) in list.iter().enumerate() {
            *scores.entry(id).or_default() += 1.0 / (k + rank as f32);
        }
    }
    let mut fused: Vec<(f32, String)> = scores
        .into_iter()
        .map(|(id, s)| (s, id.to_string()))
        .collect();
    sort_scored(&mut fused);
    fused
}

/// `k` of a fusion query: `{"fusion": "rrf"}` (Qdrant's default k = 2) or
/// `{"rrf": {"k": n}}`.
fn rrf_k(query: &Value) -> Result<f32, String> {
    if query == &json!({"fusion": "rrf"}) {
        return Ok(2.0);
    }
    let params = query
        .get("rrf")
        .and_then(Value::as_object)
        .filter(|_| query.as_object().is_some_and(|q| q.len() == 1))
        .ok_or_else(|| format!("unsupported fusion query {query}"))?;
    if let Some(key) = params.keys().find(|k| *k != "k") {
        return Err(format!("unsupported rrf parameter {key}"));
    }
    match params.get("k") {
        None => Ok(2.0),
        Some(k) => k
            .as_u64()
            .filter(|k| *k > 0)
            .map(|k| k as f32)
            .ok_or_else(|| format!("invalid rrf k {k}")),
    }
}

/// Qdrant's IDF modifier: `ln(1 + (N - n + 0.5) / (n + 0.5))` with N the points that
/// have a non-empty `name` sparse vector and n those containing `index`. Computed over
/// the whole collection, whatever the query's filter.
fn idf(coll: &Collection, name: &str, index: u32) -> f32 {
    let with_vector = coll
        .points
        .values()
        .filter_map(|p| p.sparse.get(name))
        .filter(|v| !v.is_empty());
    let (mut total, mut containing) = (0f32, 0f32);
    for v in with_vector {
        total += 1.0;
        if v.iter().any(|(i, _)| *i == index) {
            containing += 1.0;
        }
    }
    (1.0 + (total - containing + 0.5) / (containing + 0.5)).ln()
}

#[derive(Default)]
struct Collection {
    /// The creation body, reported back as `config.params`.
    params: Value,
    /// Dense vector name -> dimension (all cosine).
    dense: BTreeMap<String, usize>,
    /// Sparse vector names (all with the IDF modifier).
    sparse: Vec<String>,
    points: BTreeMap<String, Point>,
}

struct Point {
    dense: BTreeMap<String, Vec<f32>>,
    sparse: BTreeMap<String, Vec<(u32, f32)>>,
    payload: Value,
}

impl Collection {
    /// Named cosine dense vectors and IDF sparse vectors; the unnamed single-vector
    /// form and any other option are rejected.
    fn create(body: &Value) -> Result<Self, String> {
        let obj = body
            .as_object()
            .ok_or("collection config must be an object")?;
        if let Some(k) = obj
            .keys()
            .find(|k| !["vectors", "sparse_vectors"].contains(&k.as_str()))
        {
            return Err(format!("unsupported collection option {k}"));
        }
        let mut coll = Collection {
            params: body.clone(),
            ..Default::default()
        };
        let vectors = body["vectors"].as_object().ok_or("vectors must be named")?;
        for (name, cfg) in vectors {
            let size = cfg["size"]
                .as_u64()
                .filter(|_| cfg.as_object().is_some_and(|c| c.len() == 2))
                .ok_or_else(|| format!("unsupported vector config {name}: {cfg}"))?;
            if cfg["distance"] != "Cosine" {
                return Err(format!("unsupported distance for {name}: {cfg}"));
            }
            coll.dense.insert(name.clone(), size as usize);
        }
        for (name, cfg) in body["sparse_vectors"].as_object().into_iter().flatten() {
            if cfg != &json!({"modifier": "idf"}) {
                return Err(format!("unsupported sparse vector config {name}: {cfg}"));
            }
            coll.sparse.push(name.clone());
        }
        if coll.dense.is_empty() && coll.sparse.is_empty() {
            return Err("collection without vectors".into());
        }
        Ok(coll)
    }

    fn matching(&self, f: &Value) -> Result<Vec<String>, String> {
        let mut out = vec![];
        for (id, p) in &self.points {
            if f.is_null() || filter(f, id, &p.payload)? {
                out.push(id.clone());
            }
        }
        Ok(out)
    }

    fn point(&self, id: String, p: &Value) -> Result<(String, Point), String> {
        let named = p["vector"]
            .as_object()
            .ok_or_else(|| format!("point {id} needs named vectors"))?;
        let mut point = Point {
            dense: BTreeMap::new(),
            sparse: BTreeMap::new(),
            payload: p["payload"].clone(),
        };
        for (name, v) in named {
            if let Some(dim) = self.dense.get(name) {
                let v = numbers(v, "dense vector")?;
                if v.len() != *dim {
                    return Err(format!("vector {name} of size {} for {dim}", v.len()));
                }
                point.dense.insert(name.clone(), normalized(v));
            } else if self.sparse.contains(name) {
                point.sparse.insert(name.clone(), sparse(v)?);
            } else {
                return Err(format!("unknown vector name {name}"));
            }
        }
        if !point.payload.is_object() {
            return Err(format!("payload of {id} is not an object"));
        }
        Ok((id, point))
    }

    /// The stored vectors in Qdrant's response shape.
    fn vectors(&self, id: &str) -> Value {
        let p = &self.points[id];
        let mut out = serde_json::Map::new();
        for (name, v) in &p.dense {
            out.insert(name.clone(), json!(v));
        }
        for (name, v) in &p.sparse {
            let (indices, values): (Vec<u32>, Vec<f32>) = v.iter().copied().unzip();
            out.insert(name.clone(), json!({"indices": indices, "values": values}));
        }
        Value::Object(out)
    }

    /// One prefetch: the `limit` best points matching its filter by its vector.
    fn prefetch(&self, p: &Value) -> Result<Vec<String>, String> {
        let obj = p.as_object().ok_or("prefetch must be an object")?;
        if let Some(k) = obj
            .keys()
            .find(|k| !["query", "using", "limit", "filter"].contains(&k.as_str()))
        {
            return Err(format!("unsupported prefetch key {k}"));
        }
        let using = p["using"].as_str().ok_or("prefetch needs `using`")?;
        let limit = p["limit"].as_u64().unwrap_or(10) as usize;
        let candidates = self.matching(&p["filter"])?;
        let mut scored: Vec<(f32, String)> = if let Some(dim) = self.dense.get(using) {
            let q = normalized(numbers(&p["query"], "dense query")?);
            if q.len() != *dim {
                return Err(format!("query of size {} for {using} of {dim}", q.len()));
            }
            candidates
                .into_iter()
                .filter_map(|id| {
                    let v = self.points[&id].dense.get(using)?;
                    Some((dot(&q, v), id))
                })
                .collect()
        } else if self.sparse.iter().any(|s| s == using) {
            let q: Vec<(u32, f32)> = sparse(&p["query"])?
                .into_iter()
                .map(|(i, w)| (i, w * idf(self, using, i)))
                .collect();
            // Only points sharing an index with the query are returned.
            candidates
                .into_iter()
                .filter_map(|id| {
                    let v = self.points[&id].sparse.get(using)?;
                    let shared: Vec<f32> = q
                        .iter()
                        .filter_map(|(i, w)| v.iter().find(|(j, _)| j == i).map(|(_, x)| w * x))
                        .collect();
                    (!shared.is_empty()).then(|| (shared.iter().sum(), id))
                })
                .collect()
        } else {
            return Err(format!("unknown vector name {using}"));
        };
        sort_scored(&mut scored);
        Ok(scored.into_iter().take(limit).map(|(_, id)| id).collect())
    }

    /// `POST /points/query` with fused prefetches, the only form Tails sends.
    fn query(&self, body: &Value) -> Result<Value, String> {
        let obj = body.as_object().ok_or("query body must be an object")?;
        if let Some(k) = obj
            .keys()
            .find(|k| !["prefetch", "query", "limit", "with_payload"].contains(&k.as_str()))
        {
            return Err(format!("unsupported query key {k}"));
        }
        let prefetch = body["prefetch"]
            .as_array()
            .filter(|p| !p.is_empty())
            .ok_or("only fused prefetch queries are supported")?;
        let k = rrf_k(&body["query"])?;
        let limit = body["limit"].as_u64().unwrap_or(10) as usize;
        let with = body.get("with_payload").cloned().unwrap_or(json!(false));
        let lists = prefetch
            .iter()
            .map(|p| self.prefetch(p))
            .collect::<Result<Vec<_>, _>>()?;
        let points: Vec<Value> = rrf(&lists, k)
            .into_iter()
            .take(limit)
            .map(|(score, id)| {
                let payload = select_payload(&self.points[&id].payload, &with);
                json!({"id": id_value(&id), "version": 0, "score": score, "payload": payload})
            })
            .collect();
        Ok(json!({ "points": points }))
    }
}

/// `with_payload`: `true`, `false` or a list of keys to include.
fn select_payload(payload: &Value, with: &Value) -> Option<Value> {
    match with {
        Value::Bool(true) => Some(payload.clone()),
        Value::Array(keys) => {
            let mut out = serde_json::Map::new();
            for k in keys.iter().filter_map(Value::as_str) {
                if let Some(v) = payload.get(k) {
                    out.insert(k.to_string(), v.clone());
                }
            }
            Some(Value::Object(out))
        }
        _ => None,
    }
}

impl FakeQdrant {
    fn handle(&self, req: &Request) -> Result<ResponseTemplate, String> {
        let path = req.url.path().to_string();
        let parts: Vec<&str> = path.trim_matches('/').split('/').collect();
        let body: Value = if req.body.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&req.body).map_err(|e| format!("invalid JSON body: {e}"))?
        };
        let method = req.method.as_str();
        let mut collections = self.collections.lock().unwrap();
        match (method, parts.as_slice()) {
            ("PUT", ["collections", name]) => {
                collections.insert(name.to_string(), Collection::create(&body)?);
                Ok(ok(json!(true)))
            }
            ("GET", ["collections", name]) => Ok(match collections.get(*name) {
                Some(coll) => ok(json!({"status": "green", "config": {"params": coll.params}})),
                // Qdrant answers a missing collection with 404.
                None => ResponseTemplate::new(404).set_body_json(
                    json!({"status": {"error": format!("Collection `{name}` doesn't exist!")}}),
                ),
            }),
            ("DELETE", ["collections", name]) => {
                collections.remove(*name);
                Ok(ok(json!(true)))
            }
            ("PUT", ["collections", name, "points"]) => {
                let coll = collections.get_mut(*name).ok_or("collection not found")?;
                let points = body["points"].as_array().ok_or("upsert needs points")?;
                let staged = points
                    .iter()
                    .map(|p| coll.point(point_id(&p["id"])?, p))
                    .collect::<Result<Vec<_>, _>>()?;
                coll.points.extend(staged);
                Ok(ok(json!({"operation_id": 0, "status": "completed"})))
            }
            ("POST", ["collections", name, "points"]) => {
                let coll = collections.get(*name).ok_or("collection not found")?;
                let with = body.get("with_payload").cloned().unwrap_or(json!(true));
                let mut out = vec![];
                for id in body["ids"].as_array().ok_or("retrieve needs ids")? {
                    let id = point_id(id)?;
                    if let Some(p) = coll.points.get(&id) {
                        out.push(
                            json!({"id": id_value(&id), "payload": select_payload(&p.payload, &with)}),
                        );
                    }
                }
                Ok(ok(Value::Array(out)))
            }
            ("POST", ["collections", name, "points", "payload"]) => {
                let coll = collections.get_mut(*name).ok_or("collection not found")?;
                let patch = body["payload"]
                    .as_object()
                    .ok_or("set_payload needs payload")?;
                let ids: Vec<String> = body["points"]
                    .as_array()
                    .ok_or("set_payload needs points")?
                    .iter()
                    .map(point_id)
                    .collect::<Result<_, _>>()?;
                for id in ids {
                    let p = coll
                        .points
                        .get_mut(&id)
                        .ok_or_else(|| format!("set_payload on missing point {id}"))?;
                    for (k, v) in patch {
                        p.payload[k] = v.clone();
                    }
                }
                Ok(ok(json!({"operation_id": 0, "status": "completed"})))
            }
            ("POST", ["collections", name, "points", "delete"]) => {
                let coll = collections.get_mut(*name).ok_or("collection not found")?;
                let doomed = if let Some(f) = body.get("filter") {
                    coll.matching(f)?
                } else {
                    body["points"]
                        .as_array()
                        .ok_or("delete needs points or filter")?
                        .iter()
                        .map(point_id)
                        .collect::<Result<_, _>>()?
                };
                for id in doomed {
                    coll.points.remove(&id);
                }
                Ok(ok(json!({"operation_id": 0, "status": "completed"})))
            }
            ("POST", ["collections", name, "points", "count"]) => {
                let coll = collections.get(*name).ok_or("collection not found")?;
                let n = coll.matching(&body["filter"])?.len();
                Ok(ok(json!({"count": n})))
            }
            ("POST", ["collections", name, "points", "scroll"]) => {
                let coll = collections.get(*name).ok_or("collection not found")?;
                let with = body.get("with_payload").cloned().unwrap_or(json!(true));
                let with_vector = body["with_vector"].as_bool().unwrap_or(false);
                let points: Vec<Value> = coll
                    .matching(&body["filter"])?
                    .into_iter()
                    .map(|id| {
                        let payload = select_payload(&coll.points[&id].payload, &with);
                        let mut p = json!({"id": id_value(&id), "payload": payload});
                        if with_vector {
                            p["vector"] = coll.vectors(&id);
                        }
                        p
                    })
                    .collect();
                Ok(ok(json!({"points": points, "next_page_offset": null})))
            }
            ("POST", ["collections", name, "points", "query"]) => {
                let coll = collections.get(*name).ok_or("collection not found")?;
                Ok(ok(coll.query(&body)?))
            }
            _ => Err(format!("unsupported endpoint {method} {path}")),
        }
    }
}

impl Respond for FakeQdrant {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        match self.handle(req) {
            Ok(resp) => resp,
            Err(msg) => {
                let msg = format!("fake Qdrant: {msg} ({} {})", req.method, req.url.path());
                self.unsupported.lock().unwrap().push(msg.clone());
                bad_request(msg)
            }
        }
    }
}

/// Where the pipeline tests store points.
pub enum Store {
    Fake {
        server: MockServer,
        state: FakeQdrant,
        collection: String,
    },
    Real {
        endpoint: String,
        collection: String,
    },
}

impl Store {
    /// The in-memory fake with an empty collection of `dim`-sized vectors.
    pub async fn fake(dim: usize) -> Self {
        let server = MockServer::start().await;
        let state = FakeQdrant::default();
        Mock::given(wiremock::matchers::any())
            .respond_with(state.clone())
            .mount(&server)
            .await;
        let store = Self::Fake {
            server,
            state,
            collection: "tails_pipeline".into(),
        };
        store.create(dim).await;
        store
    }

    /// A fresh, uniquely named collection of `dim`-sized cosine vectors on the
    /// isolated Qdrant at `QDRANT_TEST_ENDPOINT`.
    pub async fn real(dim: usize) -> Self {
        let endpoint = std::env::var("QDRANT_TEST_ENDPOINT")
            .expect("set QDRANT_TEST_ENDPOINT to an isolated Qdrant server");
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let store = Self::Real {
            endpoint,
            collection: format!("tails_pipeline_{}_{nanos}", std::process::id()),
        };
        store.create(dim).await;
        store
    }

    /// Human-readable name for reports.
    pub fn describe(&self) -> &'static str {
        match self {
            Store::Fake { .. } => "in-memory fake Qdrant",
            Store::Real { .. } => "real Qdrant",
        }
    }

    fn endpoint(&self) -> String {
        match self {
            Store::Fake { server, .. } => server.uri(),
            Store::Real { endpoint, .. } => endpoint.clone(),
        }
    }

    fn collection(&self) -> &str {
        match self {
            Store::Fake { collection, .. } | Store::Real { collection, .. } => collection,
        }
    }

    fn http() -> reqwest::Client {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap()
    }

    /// (Re)creates the collection the way the indexer does: named `dim`-sized cosine
    /// dense vectors and IDF sparse vectors ([`Qdrant::create_collection`]).
    pub async fn create(&self, dim: usize) {
        let url = format!("{}/collections/{}", self.endpoint(), self.collection());
        let _ = Self::http().delete(&url).send().await;
        self.qdrant()
            .create_collection(dim)
            .await
            .expect("create collection");
    }

    /// A client like the indexer's and the API's, pointed at this store.
    pub fn qdrant(&self) -> Qdrant {
        let mut q = Qdrant::new(self.endpoint(), self.collection().to_string());
        q.retry = rag_core::resilience::RetryPolicy::none();
        q
    }

    /// Every stored point as `(point id, point)`, read back over the REST API, with
    /// its vectors when `with_vector`.
    async fn scroll(&self, with_vector: bool) -> Vec<(String, Value)> {
        let url = format!(
            "{}/collections/{}/points/scroll",
            self.endpoint(),
            self.collection()
        );
        let mut out = vec![];
        let mut offset = Value::Null;
        loop {
            let resp: Value = Self::http()
                .post(&url)
                .json(&json!({"limit": 256, "with_payload": true, "with_vector": with_vector, "offset": offset}))
                .send()
                .await
                .unwrap()
                .error_for_status()
                .expect("scroll")
                .json()
                .await
                .unwrap();
            for p in resp["result"]["points"].as_array().unwrap() {
                let id = match &p["id"] {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                out.push((id, p.clone()));
            }
            offset = resp["result"]["next_page_offset"].clone();
            if offset.is_null() {
                break;
            }
        }
        out
    }

    /// Every stored point as `(point id, payload)`.
    pub async fn points(&self) -> Vec<(String, Value)> {
        self.scroll(false)
            .await
            .into_iter()
            .map(|(id, p)| (id, p["payload"].clone()))
            .collect()
    }

    /// Every stored point as `(point id, named vectors)`, as Qdrant returns them.
    pub async fn vectors(&self) -> Vec<(String, Value)> {
        self.scroll(true)
            .await
            .into_iter()
            .map(|(id, p)| (id, p["vector"].clone()))
            .collect()
    }

    /// Fails on anything the fake could not serve, then drops a real collection.
    pub async fn finish(self) {
        match self {
            Store::Fake { state, .. } => {
                let unsupported = state.unsupported.lock().unwrap().clone();
                assert!(
                    unsupported.is_empty(),
                    "requests the fake Qdrant rejected:\n{}",
                    unsupported.join("\n")
                );
            }
            Store::Real { .. } => {
                let url = format!("{}/collections/{}", self.endpoint(), self.collection());
                Self::http()
                    .delete(&url)
                    .send()
                    .await
                    .unwrap()
                    .error_for_status()
                    .expect("delete collection");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload() -> Value {
        json!({"Service": "checkout", "Kind": "sLO", "Timestamp": "2026-03-11T10:15:30.000Z",
               "Tags": ["a", "b"], "Metadata": {"chunk_index": 2, "chunk_of": "x"}, "Empty": null})
    }

    #[test]
    fn fake_filters_follow_qdrant_semantics() {
        let p = payload();
        let check = |f: Value| filter(&f, "1", &p).unwrap();
        assert!(check(
            json!({"must": [{"key": "Service", "match": {"value": "checkout"}}]})
        ));
        // Keyword matches are case-sensitive, like Qdrant's.
        assert!(!check(
            json!({"must": [{"key": "Service", "match": {"value": "Checkout"}}]})
        ));
        assert!(check(
            json!({"must": [{"key": "Kind", "match": {"any": ["logs", "sLO"]}}]})
        ));
        assert!(!check(
            json!({"must": [{"key": "Kind", "match": {"any": ["slo"]}}]})
        ));
        assert!(check(
            json!({"must": [{"key": "Tags", "match": {"value": "b"}}]})
        ));
        assert!(check(
            json!({"must": [{"key": "Metadata.chunk_index", "range": {"gte": 2}}]})
        ));
        assert!(!check(
            json!({"must": [{"key": "Metadata.chunk_index", "range": {"gt": 2}}]})
        ));
        // Datetime ranges compare instants, whatever the offset notation.
        assert!(check(json!({"must": [{"key": "Timestamp", "range": {
            "gte": "2026-03-11T11:15:30+01:00", "lt": "2026-03-11T10:15:31Z"}}]})));
        assert!(!check(
            json!({"must": [{"key": "Timestamp", "range": {"lt": "2026-03-11T10:15:30Z"}}]})
        ));
        assert!(!check(
            json!({"must": [{"key": "Service", "range": {"gte": "2026-01-01T00:00:00Z"}}]})
        ));
        assert!(check(
            json!({"must": [{"is_empty": {"key": "Missing"}}, {"is_empty": {"key": "Empty"}}]})
        ));
        assert!(!check(json!({"must": [{"is_empty": {"key": "Service"}}]})));
        assert!(check(
            json!({"should": [{"key": "Service", "match": {"value": "x"}},
                                         {"must": [{"key": "Kind", "match": {"value": "sLO"}}]}]})
        ));
        assert!(!check(
            json!({"must_not": [{"key": "Service", "match": {"value": "checkout"}}]})
        ));
        assert!(check(json!({"must": [{"has_id": [1, 2]}]})));
        // Unknown conditions are errors, never silent matches.
        assert!(
            filter(
                &json!({"must": [{"key": "Text", "match": {"text": "x"}}]}),
                "1",
                &p
            )
            .is_err()
        );
        assert!(filter(&json!({"min_should": {}}), "1", &p).is_err());
    }

    #[test]
    fn rrf_sums_reciprocal_ranks_per_list() {
        let list = |ids: &[&str]| ids.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        // k = 2, 0-based ranks: a 1/2 + 1/4, b 1/3 + 1/2, c 1/4, d 1/3.
        let fused = rrf(&[list(&["a", "b", "c"]), list(&["b", "d", "a"])], 2.0);
        let want = [
            ("b", 1.0 / 3.0 + 1.0 / 2.0),
            ("a", 1.0 / 2.0 + 1.0 / 4.0),
            ("d", 1.0 / 3.0),
            ("c", 1.0 / 4.0),
        ];
        assert_eq!(fused.len(), want.len());
        for ((score, id), (want_id, want_score)) in fused.iter().zip(want) {
            assert_eq!(id, want_id);
            assert!((score - want_score).abs() < 1e-6, "{id}: {score}");
        }
        // Equal scores are ordered by point ID, numbers numerically.
        let tied = rrf(&[list(&["10", "7"]), list(&["7", "10"])], 60.0);
        assert_eq!(tied[0].1, "7");
        // `fusion: rrf` is Qdrant's default k = 2.
        assert_eq!(rrf_k(&json!({"fusion": "rrf"})).unwrap(), 2.0);
        assert_eq!(rrf_k(&json!({"rrf": {}})).unwrap(), 2.0);
        assert_eq!(rrf_k(&json!({"rrf": {"k": 60}})).unwrap(), 60.0);
        assert!(rrf_k(&json!({"fusion": "dbsf"})).is_err());
        assert!(rrf_k(&json!({"rrf": {"k": 2, "weights": [1, 2]}})).is_err());
    }

    async fn post_query(store: &Store, body: Value) -> Vec<(u64, f32)> {
        let url = format!(
            "{}/collections/{}/points/query",
            store.endpoint(),
            store.collection()
        );
        let r = Store::http().post(&url).json(&body).send().await.unwrap();
        let status = r.status();
        let v: Value = r.json().await.unwrap();
        assert!(status.is_success(), "{body} -> {status}: {v}");
        v["result"]["points"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                (
                    p["id"].as_u64().unwrap(),
                    p["score"].as_f64().unwrap() as f32,
                )
            })
            .collect()
    }

    fn assert_scores(got: &[(u64, f32)], want: &[(u64, f32)]) {
        assert_eq!(got.len(), want.len(), "{got:?} vs {want:?}");
        for ((id, score), (want_id, want_score)) in got.iter().zip(want) {
            assert_eq!(id, want_id, "{got:?} vs {want:?}");
            assert!((score - want_score).abs() < 1e-5, "{got:?} vs {want:?}");
        }
    }

    /// Sparse IDF scoring, prefetch filters and limits, and RRF fusion on a tiny
    /// collection with the production layout, against scores computed by hand. The
    /// same checks pass on Qdrant 1.19.1. No two points tie in any list, because
    /// Qdrant orders ties arbitrarily.
    async fn check_query_api(store: Store) {
        let sv = |indices: &[u32], values: &[f32]| json!({"indices": indices, "values": values});
        let point = |id: u64, dense: [f32; 2], sparse: Option<Value>, k: &str| {
            let mut vector = json!({"dense": dense});
            if let Some(s) = sparse {
                vector["sparse"] = s;
            }
            json!({"id": id, "vector": vector, "payload": {"k": k}})
        };
        let url = format!(
            "{}/collections/{}/points?wait=true",
            store.endpoint(),
            store.collection()
        );
        Store::http()
            .put(&url)
            .json(&json!({"points": [
                point(1, [1.0, 0.0], Some(sv(&[1, 2], &[1.0, 2.0])), "a"),
                point(2, [0.9, 0.1], Some(sv(&[1], &[1.0])), "b"),
                point(3, [0.0, 1.0], Some(sv(&[3], &[2.0])), "a"),
                point(4, [0.5, 0.5], Some(sv(&[1, 3], &[3.0, 1.0])), "b"),
                // Without a sparse vector: never returned by the sparse search.
                point(5, [-1.0, 0.0], None, "b"),
            ]}))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();

        // One sparse list, k = 2: the fused score of rank r is 1/(2 + r). With N = 4
        // points with a sparse vector, term 1 in 3 of them, term 2 in 1 and term 3 in 2,
        // idf = ln(1 + (N - n + 0.5)/(n + 0.5)) is 0.357, 1.204 and 0.693, and query
        // {1: 1, 2: 1, 3: 2} scores 3: 2·0.693·2 = 2.773, 1: 0.357 + 1.204·2 = 2.765,
        // 4: 0.357·3 + 2·0.693 = 2.456, 2: 0.357.
        let sparse_only = |filter: Option<Value>| {
            let mut p =
                json!({"query": sv(&[1, 2, 3], &[1.0, 1.0, 2.0]), "using": "sparse", "limit": 10});
            if let Some(f) = filter {
                p["filter"] = f;
            }
            json!({"prefetch": [p], "query": {"rrf": {"k": 2}}, "limit": 10})
        };
        assert_scores(
            &post_query(&store, sparse_only(None)).await,
            &[(3, 0.5), (1, 1.0 / 3.0), (4, 0.25), (2, 0.2)],
        );
        // The filter restricts the prefetch; the IDF stays collection-wide.
        let only_a = json!({"must": [{"key": "k", "match": {"value": "a"}}]});
        assert_scores(
            &post_query(&store, sparse_only(Some(only_a))).await,
            &[(3, 0.5), (1, 1.0 / 3.0)],
        );

        // Dense [1, 0] ranks 1, 2, 4, 3, 5; sparse {3: 1} ranks 3, 4. Fused with k = 2:
        // 3: 1/5 + 1/2, 4: 1/4 + 1/3, 1: 1/2, 2: 1/3, 5: 1/6.
        let fused = |query: Value, sparse_limit: u64, limit: u64| {
            json!({
                "prefetch": [
                    {"query": [1.0, 0.0], "using": "dense", "limit": 10},
                    {"query": sv(&[3], &[1.0]), "using": "sparse", "limit": sparse_limit}
                ],
                "query": query,
                "limit": limit,
                "with_payload": true
            })
        };
        let k2 = post_query(&store, fused(json!({"rrf": {"k": 2}}), 10, 10)).await;
        assert_scores(
            &k2,
            &[
                (3, 0.2 + 0.5),
                (4, 0.25 + 1.0 / 3.0),
                (1, 0.5),
                (2, 1.0 / 3.0),
                (5, 1.0 / 6.0),
            ],
        );
        assert_scores(
            &post_query(&store, fused(json!({"fusion": "rrf"}), 10, 10)).await,
            &k2,
        );
        // Prefetch limit (sparse keeps only 3) and top-level limit.
        assert_scores(
            &post_query(&store, fused(json!({"rrf": {"k": 2}}), 1, 3)).await,
            &[(3, 0.7), (1, 0.5), (2, 1.0 / 3.0)],
        );
        assert_scores(
            &post_query(&store, fused(json!({"rrf": {"k": 60}}), 10, 2)).await,
            &[(3, 1.0 / 63.0 + 1.0 / 60.0), (4, 1.0 / 62.0 + 1.0 / 61.0)],
        );
        store.finish().await;
    }

    #[tokio::test]
    async fn fake_query_api_matches_hand_computed_scores() {
        check_query_api(Store::fake(2).await).await;
    }

    /// `QDRANT_TEST_ENDPOINT=http://localhost:6333 cargo test -p rag-indexer --bin
    /// rag-indexer query_api_matches_real_qdrant -- --ignored`
    #[tokio::test]
    #[ignore = "requires QDRANT_TEST_ENDPOINT pointing at an isolated Qdrant server"]
    async fn query_api_matches_real_qdrant() {
        check_query_api(Store::real(2).await).await;
    }

    #[tokio::test]
    async fn fake_rejects_requests_it_does_not_implement() {
        use reqwest::Method;
        let store = Store::fake(2).await;
        let base = format!("{}/collections", store.endpoint());
        let send = |method: Method, path: &str, body: Value| {
            let url = format!("{base}/{path}");
            async move {
                Store::http()
                    .request(method, url)
                    .json(&body)
                    .send()
                    .await
                    .unwrap()
                    .status()
                    .as_u16()
            }
        };
        let points = "tails_pipeline/points";
        let query = "tails_pipeline/points/query";
        let dense_prefetch = json!([{"query": [1.0, 0.0], "using": "dense"}]);
        let bad = [
            // The old unnamed layout, other distances, sparse vectors without IDF.
            send(
                Method::PUT,
                "old",
                json!({"vectors": {"size": 2, "distance": "Cosine"}}),
            )
            .await,
            send(
                Method::PUT,
                "dot",
                json!({"vectors": {"dense": {"size": 2, "distance": "Dot"}}}),
            )
            .await,
            send(
                Method::PUT,
                "plain",
                json!({"vectors": {}, "sparse_vectors": {"sparse": {}}}),
            )
            .await,
            // Unnamed or unknown vectors on upsert.
            send(
                Method::PUT,
                points,
                json!({"points": [{"id": 1, "vector": [1.0, 0.0], "payload": {}}]}),
            )
            .await,
            send(
                Method::PUT,
                points,
                json!({"points": [{"id": 1, "vector": {"other": [1.0, 0.0]}, "payload": {}}]}),
            )
            .await,
            // The removed search endpoint, unfused and nested queries, other fusions,
            // top-level filters.
            send(
                Method::POST,
                "tails_pipeline/points/search",
                json!({"vector": [1.0, 0.0], "limit": 1}),
            )
            .await,
            send(
                Method::POST,
                query,
                json!({"query": [1.0, 0.0], "using": "dense"}),
            )
            .await,
            send(
                Method::POST,
                query,
                json!({"prefetch": [{"prefetch": dense_prefetch, "query": [1.0, 0.0], "using": "dense"}],
                       "query": {"fusion": "rrf"}}),
            )
            .await,
            send(
                Method::POST,
                query,
                json!({"prefetch": dense_prefetch, "query": {"fusion": "dbsf"}}),
            )
            .await,
            send(
                Method::POST,
                query,
                json!({"prefetch": dense_prefetch, "query": {"fusion": "rrf"}, "filter": {}}),
            )
            .await,
        ];
        assert!(bad.iter().all(|s| *s == 400), "{bad:?}");
        let Store::Fake { state, .. } = &store else {
            unreachable!()
        };
        assert_eq!(state.unsupported.lock().unwrap().len(), bad.len());
        // A missing collection is a 404, as in Qdrant, not an unsupported request.
        assert_eq!(send(Method::GET, "missing", Value::Null).await, 404);
        state.unsupported.lock().unwrap().clear();
        store.finish().await;
    }
}
