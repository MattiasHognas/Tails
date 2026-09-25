//! A fake Datadog API serving a corpus in the shape of real responses, so the real
//! adapters (`rag_core::datadog`, `service_catalog`, `change_events`) parse it:
//! monitors, dashboards (list and definition), SLOs, metrics, incidents (search,
//! timeline, attachments and postmortem notebooks), logs, service definitions and
//! change events, each with the pagination the adapter follows, plus the live
//! time-series and log queries of `/ask`.

use chrono::{DateTime, Duration, Utc};
use rag_core::datadog::Datadog;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

/// The indexing query the adapter must send for logs.
pub const LOG_INDEX_QUERY: &str = "status:error OR status:warn";
/// The indexing query the adapter must send for change events.
const CHANGE_INDEX_QUERY: &str = rag_core::change_events::DEFAULT_QUERY;

/// Datadog objects as the list/search endpoints return them.
#[derive(Debug, Clone, Default)]
pub struct Corpus {
    pub monitors: Vec<Value>,
    pub dashboards: Vec<Value>,
    pub slos: Vec<Value>,
    pub metrics: Vec<String>,
    /// Incident objects (the `data` of each search result).
    pub incidents: Vec<Value>,
    /// Log events (`{"id", "type": "log", "attributes": {...}}`).
    pub logs: Vec<Value>,
    /// Service definitions (`{"id", "type", "attributes": {"schema", "meta"}}`).
    pub service_definitions: Vec<Value>,
    /// Events (`{"id", "type": "event", "attributes": {...}}`) matching the change query.
    pub events: Vec<Value>,
    /// Timeline cells per incident ID (`GET /api/v2/incidents/{id}/timeline`).
    pub incident_timelines: HashMap<String, Vec<Value>>,
    /// Attachments per incident ID (`GET /api/v2/incidents/{id}/attachments`).
    pub incident_attachments: HashMap<String, Vec<Value>>,
    /// Notebooks (the `data` of `GET /api/v1/notebooks/{id}`).
    pub notebooks: Vec<Value>,
    /// Incident IDs whose timeline and attachment requests fail with a 500.
    pub failing_incident_details: Vec<String>,
}

/// The keys of a dashboard list entry; the rest of a corpus dashboard (widgets, template
/// variables) is only returned by `GET /api/v1/dashboard/{id}`.
const DASHBOARD_SUMMARY_KEYS: [&str; 10] = [
    "id",
    "title",
    "description",
    "layout_type",
    "url",
    "is_read_only",
    "created_at",
    "modified_at",
    "author_handle",
    "deleted_at",
];

fn dashboard_summary(d: &Value) -> Value {
    let mut out = serde_json::Map::new();
    for k in DASHBOARD_SUMMARY_KEYS {
        if let Some(v) = d.get(k) {
            out.insert(k.to_string(), v.clone());
        }
    }
    Value::Object(out)
}

fn by_incident(v: &Value) -> HashMap<String, Vec<Value>> {
    v.as_object()
        .map(|m| {
            m.iter()
                .map(|(k, v)| (k.clone(), v.as_array().cloned().unwrap_or_default()))
                .collect()
        })
        .unwrap_or_default()
}

pub fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rag-core/tests/fixtures/datadog")
}

fn read_json(path: PathBuf) -> Value {
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{path:?}: {e}"))
}

impl Corpus {
    /// Every recorded response in `rag-core/tests/fixtures/datadog`.
    pub fn fixtures() -> Self {
        Self::fixtures_from(&fixture_dir())
    }

    /// Every recorded response in `dir` (a copy of `rag-core/tests/fixtures/datadog`).
    pub fn fixtures_from(dir: &std::path::Path) -> Self {
        let f = |name: &str| read_json(dir.join(name));
        let array = |v: &Value| v.as_array().cloned().unwrap_or_default();
        let incidents = ["incidents_search_page1.json", "incidents_search_page2.json"]
            .iter()
            .flat_map(|p| array(&f(p)["data"]["attributes"]["incidents"]))
            .map(|i| i["data"].clone())
            .collect();
        let logs = ["logs_search_page1.json", "logs_search_page2.json"]
            .iter()
            .flat_map(|p| array(&f(p)["data"]))
            .collect();
        let service_definitions = [
            "service_definitions_page1.json",
            "service_definitions_page2.json",
            "service_definitions_schema_versions.json",
        ]
        .iter()
        .flat_map(|p| array(&f(p)["data"]))
        .collect();
        let events = ["events_search_page1.json", "events_search_page2.json"]
            .iter()
            .flat_map(|p| array(&f(p)["data"]))
            .collect();
        let mut dashboards = array(&f("dashboards.json")["dashboards"]);
        dashboards.push(f("dashboard_get.json"));
        Self {
            monitors: array(&f("monitors.json")),
            dashboards,
            slos: array(&f("slos.json")["data"]),
            metrics: array(&f("metrics.json")["metrics"])
                .iter()
                .filter_map(|m| m.as_str().map(str::to_string))
                .collect(),
            incidents,
            logs,
            service_definitions,
            events,
            ..Self::default()
        }
    }

    /// A corpus file with the same top-level keys (see
    /// `tests/incident_questions/corpus.json`), plus `logBursts`: runs of near-identical
    /// logs, expanded by [`expand_burst`] and appended to `logs`; `incidentTimelines` and
    /// `incidentAttachments` (per incident ID) and `notebooks`.
    pub fn from_json(v: &Value) -> Self {
        let array = |k: &str| v[k].as_array().cloned().unwrap_or_default();
        let mut logs = array("logs");
        logs.extend(array("logBursts").iter().flat_map(expand_burst));
        Self {
            monitors: array("monitors"),
            dashboards: array("dashboards"),
            slos: array("slos"),
            metrics: array("metrics")
                .iter()
                .filter_map(|m| m.as_str().map(str::to_string))
                .collect(),
            incidents: array("incidents"),
            logs,
            service_definitions: array("serviceDefinitions"),
            events: array("events"),
            incident_timelines: by_incident(&v["incidentTimelines"]),
            incident_attachments: by_incident(&v["incidentAttachments"]),
            notebooks: array("notebooks"),
            failing_incident_details: vec![],
        }
    }

    /// The ID of the log pattern day document that holds Datadog log `raw_id` of this corpus,
    /// computed with the adapter's own parsing and grouping.
    pub fn log_doc_id(&self, raw_id: &str) -> Option<String> {
        let log = self.logs.iter().find(|l| l["id"] == raw_id)?;
        let event = rag_core::datadog::log_event(log)?;
        Some(rag_core::log_patterns::group(&[event])[0].doc_id())
    }

    /// `id` with a `log_<raw id>` reference to a corpus log replaced by the ID of the
    /// pattern day document that holds it; anything else is returned unchanged.
    pub fn resolve(&self, id: &str) -> String {
        id.strip_prefix("log_")
            .and_then(|raw| self.log_doc_id(raw))
            .unwrap_or_else(|| id.to_string())
    }

    pub fn extend(&mut self, other: Corpus) {
        self.monitors.extend(other.monitors);
        self.dashboards.extend(other.dashboards);
        self.slos.extend(other.slos);
        self.metrics.extend(other.metrics);
        self.incidents.extend(other.incidents);
        self.logs.extend(other.logs);
        self.service_definitions.extend(other.service_definitions);
        self.events.extend(other.events);
        self.incident_timelines.extend(other.incident_timelines);
        self.incident_attachments.extend(other.incident_attachments);
        self.notebooks.extend(other.notebooks);
        self.failing_incident_details
            .extend(other.failing_incident_details);
    }
}

/// `count` logs `<idPrefix><i>`, one every `everySeconds` from `start`, with `service`,
/// `env` and `status`. In `message`, `{ms}` becomes a varying duration and `{uuid}` a
/// UUID unique to the log, the way real bursts differ only in numbers and IDs.
pub fn expand_burst(spec: &Value) -> Vec<Value> {
    let s = |k: &str| {
        spec[k]
            .as_str()
            .unwrap_or_else(|| panic!("logBursts: missing {k}"))
            .to_string()
    };
    let n = |k: &str| {
        spec[k]
            .as_u64()
            .unwrap_or_else(|| panic!("logBursts: missing {k}"))
    };
    let start = ts(&spec["start"]).expect("logBursts: start");
    (0..n("count"))
        .map(|i| {
            let at = start + Duration::seconds((i * n("everySeconds")) as i64);
            let message = s("message")
                .replace("{ms}", &(900 + (i * 37) % 2100).to_string())
                .replace(
                    "{uuid}",
                    &format!("{:08x}-7425-40de-944b-{:012x}", i * 7919, i),
                );
            json!({"id": format!("{}{i}", s("idPrefix")), "type": "log", "attributes": {
                "service": s("service"), "status": s("status"), "message": message,
                "timestamp": at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                "tags": [format!("env:{}", s("env"))]
            }})
        })
        .collect()
}

fn param(req: &Request, name: &str) -> Option<usize> {
    req.url
        .query_pairs()
        .find(|(k, _)| k == name)
        .and_then(|(_, v)| v.parse().ok())
}

fn page(items: &[Value], offset: usize, size: usize) -> Vec<Value> {
    items.iter().skip(offset).take(size).cloned().collect()
}

fn ts(v: &Value) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(v.as_str()?)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

/// The indexing endpoints of the fake Datadog API over one corpus.
#[derive(Clone)]
pub struct IndexApi(Arc<Corpus>);

impl IndexApi {
    pub fn new(corpus: Corpus) -> Self {
        Self(Arc::new(corpus))
    }

    fn handle(&self, req: &Request) -> Result<ResponseTemplate, String> {
        if req.headers.get("DD-API-KEY").is_none()
            || req.headers.get("DD-APPLICATION-KEY").is_none()
        {
            return Ok(ResponseTemplate::new(403));
        }
        let c = &self.0;
        let json = |v: Value| Ok(ResponseTemplate::new(200).set_body_json(v));
        match (req.method.as_str(), req.url.path()) {
            ("GET", "/api/v1/monitor") => {
                let size = param(req, "page_size").ok_or("page_size")?;
                let page_no = param(req, "page").ok_or("page")?;
                json(Value::Array(page(&c.monitors, page_no * size, size)))
            }
            ("GET", "/api/v1/dashboard") => {
                let (start, count) = (
                    param(req, "start").ok_or("start")?,
                    param(req, "count").ok_or("count")?,
                );
                let summaries: Vec<Value> = page(&c.dashboards, start, count)
                    .iter()
                    .map(dashboard_summary)
                    .collect();
                json(json!({"dashboards": summaries}))
            }
            ("GET", p) if p.starts_with("/api/v1/dashboard/") => {
                let id = &p["/api/v1/dashboard/".len()..];
                match c.dashboards.iter().find(|d| d["id"] == id) {
                    Some(d) => json(d.clone()),
                    None => Ok(ResponseTemplate::new(404)),
                }
            }
            ("GET", p) if p.starts_with("/api/v1/notebooks/") => {
                let id: u64 = p["/api/v1/notebooks/".len()..]
                    .parse()
                    .map_err(|_| "notebook id")?;
                match c.notebooks.iter().find(|n| n["id"] == id) {
                    Some(n) => json(json!({"data": n})),
                    None => Ok(ResponseTemplate::new(404)),
                }
            }
            ("GET", p)
                if p.starts_with("/api/v2/incidents/") && p != "/api/v2/incidents/search" =>
            {
                let rest = &p["/api/v2/incidents/".len()..];
                let (id, what) = rest.split_once('/').ok_or("incident sub-resource")?;
                if c.failing_incident_details.iter().any(|f| f == id) {
                    return Ok(ResponseTemplate::new(500));
                }
                let items = match what {
                    "timeline" => c.incident_timelines.get(id),
                    "attachments" => c.incident_attachments.get(id),
                    _ => return Err(format!("unsupported incident resource {what}")),
                };
                json(json!({"data": items.cloned().unwrap_or_default()}))
            }
            ("GET", "/api/v1/slo") => {
                let (offset, limit) = (
                    param(req, "offset").ok_or("offset")?,
                    param(req, "limit").ok_or("limit")?,
                );
                json(json!({"data": page(&c.slos, offset, limit), "error": null}))
            }
            ("GET", "/api/v1/metrics") => {
                param(req, "from").ok_or("from")?;
                json(json!({"metrics": c.metrics}))
            }
            ("GET", "/api/v2/incidents/search") => {
                let mut sorted = c.incidents.clone();
                sorted.sort_by_key(|i| std::cmp::Reverse(ts(&i["attributes"]["created"])));
                let offset = param(req, "page[offset]").ok_or("page[offset]")?;
                let size = param(req, "page[size]").ok_or("page[size]")?;
                let items: Vec<Value> = page(&sorted, offset, size)
                    .into_iter()
                    .map(|i| json!({"data": i}))
                    .collect();
                let next = offset + items.len();
                json(json!({
                    "data": {"type": "incidents_search_results",
                             "attributes": {"facets": {}, "incidents": items, "total": sorted.len()}},
                    "meta": {"pagination": {"offset": offset, "next_offset": next, "size": items.len()}}
                }))
            }
            ("POST", "/api/v2/logs/events/search") => {
                let body: Value = serde_json::from_slice(&req.body).map_err(|e| e.to_string())?;
                let f = &body["filter"];
                if f["query"] != LOG_INDEX_QUERY {
                    return Err(format!("unexpected log query {}", f["query"]));
                }
                let (from, to) = (ts(&f["from"]).ok_or("from")?, ts(&f["to"]).ok_or("to")?);
                let mut logs: Vec<Value> = c
                    .logs
                    .iter()
                    .filter(|l| {
                        ts(&l["attributes"]["timestamp"]).is_some_and(|t| t >= from && t <= to)
                    })
                    .cloned()
                    .collect();
                logs.sort_by_key(|l| ts(&l["attributes"]["timestamp"]));
                let offset: usize = body["page"]["cursor"]
                    .as_str()
                    .map(|c| c.parse().map_err(|_| "bad cursor"))
                    .transpose()?
                    .unwrap_or(0);
                let limit = body["page"]["limit"].as_u64().ok_or("limit")? as usize;
                let data = page(&logs, offset, limit);
                let next = offset + data.len();
                let mut resp = json!({"data": data, "meta": {"page": {}}});
                if next < logs.len() {
                    resp["meta"]["page"]["after"] = json!(next.to_string());
                }
                json(resp)
            }
            ("GET", "/api/v2/services/definitions") => {
                let size = param(req, "page[size]").ok_or("page[size]")?;
                let number = param(req, "page[number]").ok_or("page[number]")?;
                if size > 100 {
                    return Err(format!("page[size] {size} above the maximum 100"));
                }
                json(json!({"data": page(&c.service_definitions, number * size, size)}))
            }
            ("POST", "/api/v2/events/search") => {
                let body: Value = serde_json::from_slice(&req.body).map_err(|e| e.to_string())?;
                let f = &body["filter"];
                if f["query"] != CHANGE_INDEX_QUERY {
                    return Err(format!("unexpected event query {}", f["query"]));
                }
                if body["sort"] != "timestamp" {
                    return Err(format!("unexpected event sort {}", body["sort"]));
                }
                let (from, to) = (ts(&f["from"]).ok_or("from")?, ts(&f["to"]).ok_or("to")?);
                let mut events: Vec<Value> = c
                    .events
                    .iter()
                    .filter(|e| {
                        ts(&e["attributes"]["timestamp"]).is_some_and(|t| t >= from && t <= to)
                    })
                    .cloned()
                    .collect();
                events.sort_by_key(|e| ts(&e["attributes"]["timestamp"]));
                let offset: usize = body["page"]["cursor"]
                    .as_str()
                    .map(|c| c.parse().map_err(|_| "bad cursor"))
                    .transpose()?
                    .unwrap_or(0);
                let limit = body["page"]["limit"].as_u64().ok_or("limit")? as usize;
                if limit > 1000 {
                    return Err(format!("page.limit {limit} above the maximum 1000"));
                }
                let data = page(&events, offset, limit);
                let next = offset + data.len();
                let mut resp = json!({"data": data, "meta": {"page": {}}});
                if next < events.len() {
                    resp["meta"]["page"]["after"] = json!(next.to_string());
                }
                json(resp)
            }
            (m, p) => Err(format!("unsupported endpoint {m} {p}")),
        }
    }
}

impl Respond for IndexApi {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        self.handle(req).unwrap_or_else(|msg| {
            ResponseTemplate::new(400)
                .set_body_json(json!({"errors": [format!("fake Datadog: {msg}")]}))
        })
    }
}

/// Serves `corpus` like the Datadog API and returns a client for it.
pub async fn serve(corpus: Corpus) -> (MockServer, Datadog) {
    let server = MockServer::start().await;
    Mock::given(wiremock::matchers::any())
        .respond_with(IndexApi::new(corpus))
        .mount(&server)
        .await;
    let dd = client(&server);
    (server, dd)
}

pub fn client(server: &MockServer) -> Datadog {
    let mut dd = Datadog::new("api-key".into(), "app-key".into(), "datadoghq.eu".into());
    dd.api_base = server.uri();
    dd.retry = rag_core::resilience::RetryPolicy::none();
    dd
}

/// Live data for one question: series per metric query and error/warn logs per log
/// query. Unknown queries return an empty series or no logs.
#[derive(Debug, Clone, Default)]
pub struct Live {
    /// `avg:metric{service:x,env:y}` -> hourly points `(unix ms, value)`.
    pub series: HashMap<String, Vec<(i64, f64)>>,
    /// `service:x env:y status:(error OR warn)` -> log events.
    pub logs: HashMap<String, Vec<Value>>,
}

impl Live {
    /// Hourly points from `from` to `to` at `baseline`, with `spikes` (hour start,
    /// value) replacing the baseline value.
    pub fn hourly(
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        baseline: f64,
        spikes: &[(DateTime<Utc>, f64)],
    ) -> Vec<(i64, f64)> {
        let mut out = vec![];
        let mut t = from;
        while t < to {
            let v = spikes
                .iter()
                .find(|(at, _)| *at == t)
                .map_or(baseline, |(_, v)| *v);
            out.push((t.timestamp_millis(), v));
            t += Duration::hours(1);
        }
        out
    }
}

/// The live-evidence endpoints of the fake Datadog API (`/api/v1/query` and log
/// searches) over one question's [`Live`] data.
#[derive(Clone)]
pub struct LiveApi(Arc<Live>);

impl LiveApi {
    pub fn new(live: Live) -> Self {
        Self(Arc::new(live))
    }
}

impl Respond for LiveApi {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        match req.url.path() {
            "/api/v1/query" => {
                let arg = |name: &str| {
                    req.url
                        .query_pairs()
                        .find(|(k, _)| k == name)
                        .map(|(_, v)| v.to_string())
                        .unwrap_or_default()
                };
                let query = arg("query");
                let secs = |name: &str| arg(name).parse::<i64>().unwrap_or_default() * 1000;
                let (from, to) = (secs("from"), secs("to"));
                let series: Vec<Value> = self
                    .0
                    .series
                    .get(&query)
                    .map(|points| {
                        let metric = query.split(['{', ':']).nth(1).unwrap_or_default();
                        vec![json!({
                            "metric": metric, "expression": query, "scope": "",
                            "interval": 3600,
                            "pointlist": points
                                .iter()
                                .filter(|(t, _)| (from..=to).contains(t))
                                .map(|(t, v)| json!([t, v]))
                                .collect::<Vec<_>>()
                        })]
                    })
                    .unwrap_or_default();
                ResponseTemplate::new(200).set_body_json(json!({"status": "ok", "series": series}))
            }
            "/api/v2/logs/events/search" => {
                let body: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
                let query = body["filter"]["query"].as_str().unwrap_or_default();
                let data = self.0.logs.get(query).cloned().unwrap_or_default();
                ResponseTemplate::new(200)
                    .set_body_json(json!({"data": data, "meta": {"page": {}}}))
            }
            _ => ResponseTemplate::new(404),
        }
    }
}

/// Serves `live` for the API's live-evidence queries.
pub async fn serve_live(live: Live) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(wiremock::matchers::any())
        .respond_with(LiveApi::new(live))
        .mount(&server)
        .await;
    server
}
