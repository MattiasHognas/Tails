//! Vector stores for the pipeline tests: an in-memory fake that implements the
//! Qdrant REST subset Tails uses, or a real Qdrant (`QDRANT_TEST_ENDPOINT`).
//!
//! The fake stores the JSON the writer actually sent and evaluates the reader's
//! filters itself, so a key name, value or format that only one side changed makes
//! a test fail. Anything it does not implement (an endpoint, a filter condition)
//! is recorded and fails [`Store::finish`] rather than silently matching.

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use rag_core::qdrant::Qdrant;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

#[derive(Default)]
struct Collection {
    dim: Option<usize>,
    /// Point ID (canonical string) -> (vector, payload).
    points: BTreeMap<String, (Vec<f32>, Value)>,
}

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

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
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
    fn matching(&self, coll: &Collection, f: &Value) -> Result<Vec<String>, String> {
        let mut out = vec![];
        for (id, (_, payload)) in &coll.points {
            if f.is_null() || filter(f, id, payload)? {
                out.push(id.clone());
            }
        }
        Ok(out)
    }

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
                let dim = body["vectors"]["size"].as_u64().map(|d| d as usize);
                collections.insert(
                    name.to_string(),
                    Collection {
                        dim,
                        ..Default::default()
                    },
                );
                Ok(ok(json!(true)))
            }
            ("DELETE", ["collections", name]) => {
                collections.remove(*name);
                Ok(ok(json!(true)))
            }
            ("PUT", ["collections", name, "points"]) => {
                let coll = collections.get_mut(*name).ok_or("collection not found")?;
                let points = body["points"].as_array().ok_or("upsert needs points")?;
                let mut staged = vec![];
                for p in points {
                    let id = point_id(&p["id"])?;
                    let vector: Vec<f32> = p["vector"]
                        .as_array()
                        .ok_or("point needs a dense vector")?
                        .iter()
                        .map(|x| x.as_f64().map(|x| x as f32).ok_or("vector must be numbers"))
                        .collect::<Result<_, _>>()?;
                    let dim = *coll.dim.get_or_insert(vector.len());
                    if vector.len() != dim {
                        return Err(format!(
                            "vector of size {} for dimension {dim}",
                            vector.len()
                        ));
                    }
                    if !p["payload"].is_object() {
                        return Err(format!("payload of {id} is not an object"));
                    }
                    staged.push((id, (vector, p["payload"].clone())));
                }
                coll.points.extend(staged);
                Ok(ok(json!({"operation_id": 0, "status": "completed"})))
            }
            ("POST", ["collections", name, "points"]) => {
                let coll = collections.get(*name).ok_or("collection not found")?;
                let with = body.get("with_payload").cloned().unwrap_or(json!(true));
                let mut out = vec![];
                for id in body["ids"].as_array().ok_or("retrieve needs ids")? {
                    let id = point_id(id)?;
                    if let Some((_, payload)) = coll.points.get(&id) {
                        out.push(
                            json!({"id": id_value(&id), "payload": select_payload(payload, &with)}),
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
                    let (_, payload) = coll
                        .points
                        .get_mut(&id)
                        .ok_or_else(|| format!("set_payload on missing point {id}"))?;
                    for (k, v) in patch {
                        payload[k] = v.clone();
                    }
                }
                Ok(ok(json!({"operation_id": 0, "status": "completed"})))
            }
            ("POST", ["collections", name, "points", "delete"]) => {
                let coll = collections.get_mut(*name).ok_or("collection not found")?;
                let doomed = if let Some(f) = body.get("filter") {
                    self.matching(coll, f)?
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
                let n = self.matching(coll, &body["filter"])?.len();
                Ok(ok(json!({"count": n})))
            }
            ("POST", ["collections", name, "points", "scroll"]) => {
                let coll = collections.get(*name).ok_or("collection not found")?;
                let with = body.get("with_payload").cloned().unwrap_or(json!(true));
                let points: Vec<Value> = self
                    .matching(coll, &body["filter"])?
                    .into_iter()
                    .map(|id| {
                        let payload = select_payload(&coll.points[&id].1, &with);
                        json!({"id": id_value(&id), "payload": payload})
                    })
                    .collect();
                Ok(ok(json!({"points": points, "next_page_offset": null})))
            }
            ("POST", ["collections", name, "points", "search"]) => {
                let coll = collections.get(*name).ok_or("collection not found")?;
                let vector: Vec<f32> = body["vector"]
                    .as_array()
                    .ok_or("search needs a vector")?
                    .iter()
                    .filter_map(|x| x.as_f64().map(|x| x as f32))
                    .collect();
                if coll.dim.is_some_and(|d| d != vector.len()) {
                    return Err("query vector has the wrong dimension".into());
                }
                let limit = body["limit"].as_u64().ok_or("search needs a limit")? as usize;
                let with = body.get("with_payload").cloned().unwrap_or(json!(false));
                let mut scored: Vec<(f32, String)> = self
                    .matching(coll, &body["filter"])?
                    .into_iter()
                    .map(|id| (cosine(&vector, &coll.points[&id].0), id))
                    .collect();
                scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
                let result: Vec<Value> = scored
                    .into_iter()
                    .take(limit)
                    .map(|(score, id)| {
                        let payload = select_payload(&coll.points[&id].1, &with);
                        json!({"id": id_value(&id), "version": 0, "score": score, "payload": payload})
                    })
                    .collect();
                Ok(ok(Value::Array(result)))
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

    /// (Re)creates the collection for `dim`-sized cosine vectors.
    pub async fn create(&self, dim: usize) {
        let url = format!("{}/collections/{}", self.endpoint(), self.collection());
        let _ = Self::http().delete(&url).send().await;
        Self::http()
            .put(&url)
            .json(&json!({"vectors": {"size": dim, "distance": "Cosine"}}))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .expect("create collection");
    }

    /// A client like the indexer's and the API's, pointed at this store.
    pub fn qdrant(&self) -> Qdrant {
        let mut q = Qdrant::new(self.endpoint(), self.collection().to_string());
        q.retry = rag_core::resilience::RetryPolicy::none();
        q
    }

    /// Every stored point as `(point id, payload)`, read back over the REST API.
    pub async fn points(&self) -> Vec<(String, Value)> {
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
                .json(&json!({"limit": 256, "with_payload": true, "with_vector": false, "offset": offset}))
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
                out.push((id, p["payload"].clone()));
            }
            offset = resp["result"]["next_page_offset"].clone();
            if offset.is_null() {
                break;
            }
        }
        out
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
}
