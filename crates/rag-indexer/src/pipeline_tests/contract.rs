//! Writer/reader agreement: index the recorded Datadog fixtures plus a small crafted
//! corpus with the real indexer, then ask the real API and check that the stored
//! payload, the planner's scope and the retrieval filter agree.
//!
//! Drift each check catches:
//! - payload keys or casing (`Title` vs `title`, lowercase `id`): the reader's
//!   `QdrantPayload` must decode every stored point to the document the adapters
//!   produced ([`check_stored_points`]);
//! - `Kind` values: the writer's value for every kind must be the filter's value, and
//!   kind-restricted questions must return exactly that kind (`sLO` today);
//! - `Timestamp` format vs the datetime `range`: in-window events are returned,
//!   events just before the window and at its (exclusive) end are not;
//! - `Service`/`Environment` casing vs the lowercased planner and explicit values
//!   (`Auth-API` from Datadog, `AUTH-API` from the planner, `Checkout` from a caller);
//! - `Metadata.chunk_of` and `Metadata.pattern_id` vs the reranker's grouping: a
//!   multi-chunk log pattern is one source, and so are the days of one pattern;
//! - log patterns: logs are stored grouped by pattern and UTC day; `Metadata.first_seen`
//!   is RFC 3339, and a day whose first log is before a window and last after it is
//!   retrieved although its `Timestamp` (last log) is after the window, while a day with
//!   no log in the window's hours is not; the prompt counts a pattern per day in the
//!   asker's timezone;
//! - point IDs: every point ID is the UUIDv5 of its logical chunk ID;
//! - vector names and inputs: every point has the dense and the sparse vector under the
//!   names the reader's hybrid query uses, the sparse one built from the same embedding
//!   input (header + chunk) as the dense one ([`check_stored_vectors`]);
//! - bookkeeping keys (`ContentHash`, `ChunkCount`, `SyncId`) are written but never
//!   reach `RagDocument` or `sources`; a second run leaves unchanged documents alone;
//! - `sources` point at stored documents, are numbered like the prompt, and every
//!   citation in the answer resolves (unknown ones are reported in `citationWarnings`);
//! - incident details and dashboard definitions: an incident's timeline and postmortem
//!   and a dashboard's widgets reach the stored text, a dashboard's service from its
//!   widget queries (`Auth-API`) matches the planner's `AUTH-API`, an incident whose
//!   detail requests fail is still stored, and the second run fetches no dashboard
//!   definition again.

use super::support::{self, Corpus, FakeOpenAi, Store, at, openai::DIM};
use rag_core::{
    chunk::embedding_input,
    domain::{RagDocument, SourceKind},
    qdrant::{
        CHUNK_COUNT_KEY, CONTENT_HASH_KEY, DENSE_VECTOR, SPARSE_VECTOR, SYNC_ID_KEY, point_id,
    },
    retrieval::RetrievalScope,
    sparse::document_vector,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

const NOW: &str = "2026-03-12T09:00:00Z";
const TZ: &str = "Europe/Stockholm";

/// A long message with multibyte text at every position. Variants share their first
/// 160 chars, so they are one pattern with `variant` in each sample.
fn long_message(variant: &str) -> String {
    let unit = "Återförsök mot bankgatewayen misslyckades 💳 決済エラー; e\u{0301}tat inconnu. ";
    format!(
        "{}{variant} {}",
        unit.repeat(3),
        unit.repeat(4000 / unit.chars().count() + 1)
    )
}

fn crafted() -> Corpus {
    let log = |id: &str, service: &str, env: &str, ts: &str, msg: &str| {
        json!({"id": id, "type": "log", "attributes": {
            "service": service, "status": "error", "timestamp": ts,
            "message": msg, "tags": [format!("env:{env}")]
        }})
    };
    Corpus::from_json(&json!({
        "monitors": [
            {"id": 7001, "type": "query alert", "name": "Checkout – svarstid över 1,5 s ⏱",
             "query": "avg(last_5m):avg:trace.http.request.duration{service:checkout,env:prod} > 1.5",
             "message": "Kontrollera anslutningspoolen.", "tags": ["service:checkout", "env:prod"]},
            {"id": 7002, "type": "query alert", "name": "Checkout latency (staging)",
             "query": "avg(last_5m):avg:trace.http.request.duration{service:checkout,env:staging} > 1.5",
             "message": "Staging only.", "tags": ["service:checkout", "env:staging"]}
        ],
        "slos": [
            {"id": "slo-checkout", "name": "Checkout availability", "description": "",
             "type": "metric", "thresholds": [{"timeframe": "30d", "target": 99.9}],
             "tags": ["service:checkout", "env:prod"]}
        ],
        "metrics": ["checkout.orders.completed"],
        "dashboards": [
            {"id": "dash-auth", "title": "Inloggning – översikt 🔐", "description": null,
             "created_at": "2025-05-01T10:00:00+00:00", "modified_at": "2026-01-01T10:00:00+00:00",
             "author_handle": "id@example.com", "template_variables": null,
             "widgets": [
                {"id": 1, "definition": {"type": "timeseries", "title": "Misslyckade inloggningar",
                    "requests": [{"q": "sum:auth.login.failures{service:Auth-API,env:PROD}.as_count()"}]}}
             ]}
        ],
        "incidents": [
            {"id": "inc-auth", "type": "incidents", "attributes": {
                "public_id": 901, "title": "Auth-API login failures",
                "created": "2026-03-11T14:00:00+00:00", "state": "resolved",
                "customer_impact_scope": "Users could not log in",
                "fields": {"severity": {"type": "dropdown", "value": "SEV-1"},
                           "services": {"type": "autocomplete", "value": ["Auth-API"]},
                           "env": {"type": "dropdown", "value": "PROD"}}}},
            {"id": "inc-flaky", "type": "incidents", "attributes": {
                "public_id": 902, "title": "Checkout flaky retries",
                "created": "2026-03-11T15:00:00+00:00", "state": "resolved",
                "fields": {"services": {"type": "autocomplete", "value": ["checkout"]},
                           "env": {"type": "dropdown", "value": "prod"}}}}
        ],
        "incidentTimelines": {"inc-auth": [
            {"id": "c1", "type": "incident_timeline_cells", "attributes": {"cell_type": "markdown",
             "created": "2026-03-11T14:20:00+00:00",
             "content": {"content": "Roterade signeringsnyckeln tillbaka 🔑; inloggningar återställda."}}}
        ]},
        "incidentAttachments": {"inc-auth": [
            {"id": "a1", "type": "incident_attachments", "attributes": {"attachment_type": "postmortem",
             "attachment": {"title": "Postmortem 901", "documentUrl": "https://app.datadoghq.eu/notebook/901/pm"}}}
        ]},
        "notebooks": [
            {"id": 901, "type": "notebooks", "attributes": {"name": "Postmortem 901", "cells": [
                {"id": "x", "type": "notebook_cells", "attributes": {"definition": {"type": "markdown",
                 "text": "Grundorsak: nyckelrotation utan överlapp."}}}
            ]}}
        ],
        "logs": [
            log("auth-1", "Auth-API", "prod", "2026-03-11T14:02:00.000Z", "login failed för åsa.öberg"),
            // One pattern: numbers are masked.
            log("co-in", "checkout", "prod", "2026-03-11T10:15:30.123Z", "db connection pool exhausted: 50/50 in use"),
            log("co-in-2", "checkout", "prod", "2026-03-11T10:16:00.000Z", "db connection pool exhausted: 48/50 in use"),
            log("co-before", "checkout", "prod", "2026-03-10T22:59:59.000Z", "payment callback rejected"),
            log("co-at-end", "checkout", "prod", "2026-03-11T23:00:00.000Z", "cart cache flush failed"),
            // One UTC day: first log before a 10:00..12:00 window, last log after it.
            log("co-span-1", "checkout", "prod", "2026-03-11T09:30:00.000Z", "inventory lookup slow"),
            log("co-span-2", "checkout", "prod", "2026-03-11T10:45:00.000Z", "inventory lookup slow"),
            log("co-span-3", "checkout", "prod", "2026-03-11T12:30:00.000Z", "inventory lookup slow"),
            // Its span overlaps that window, but no log falls in the window's hours.
            log("co-gap-1", "checkout", "prod", "2026-03-11T09:00:00.000Z", "search index stale"),
            log("co-gap-2", "checkout", "prod", "2026-03-11T13:00:00.000Z", "search index stale"),
            // Two UTC days of one pattern, both yesterday in Stockholm.
            log("co-night-1", "checkout", "prod", "2026-03-10T23:30:00.000Z", "payment webhook retry exhausted"),
            log("co-night-2", "checkout", "prod", "2026-03-11T00:30:00.000Z", "payment webhook retry exhausted"),
            log("co-staging", "checkout", "staging", "2026-03-11T10:20:00.000Z", "db connection pool exhausted: 10/10 in use"),
            log("long", "payments", "prod", "2026-03-11T12:00:00.000Z", &long_message("första")),
            log("long-2", "payments", "prod", "2026-03-11T12:05:00.000Z", &long_message("andra")),
        ]
    }))
}

fn corpus() -> Corpus {
    let mut c = Corpus::fixtures();
    c.extend(crafted());
    c.failing_incident_details = vec!["inc-flaky".into()];
    c
}

/// Every point decodes to exactly the chunk the adapters and chunker produce, under the
/// UUIDv5 of its logical ID, with bookkeeping the reader never exposes.
async fn check_stored_points(store: &Store, expected: &BTreeMap<String, RagDocument>) {
    let stored = support::stored_chunks(store).await;
    assert_eq!(
        stored.keys().collect::<Vec<_>>(),
        expected.keys().collect::<Vec<_>>(),
        "stored chunk IDs differ from the adapters' chunks"
    );
    let chunk_counts: BTreeMap<&str, usize> =
        expected.values().fold(BTreeMap::new(), |mut m, d| {
            *m.entry(d.parent_id()).or_default() += 1;
            m
        });
    for (id, s) in &stored {
        let want = &expected[id];
        assert_eq!(
            serde_json::to_value(&s.doc).unwrap(),
            serde_json::to_value(want).unwrap(),
            "payload of {id} does not round-trip"
        );
        assert_eq!(s.point_id, point_id(id).to_string(), "point ID of {id}");

        // Keys the writer sends: the reader's schema plus indexer bookkeeping only.
        let keys: BTreeSet<&str> = s
            .raw
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        let mut allowed: BTreeSet<&str> = [
            "id",
            "Title",
            "Text",
            "SourceUri",
            "Kind",
            "Timestamp",
            "Service",
            "Environment",
            "Metadata",
            CONTENT_HASH_KEY,
            CHUNK_COUNT_KEY,
            SYNC_ID_KEY,
        ]
        .into();
        if !matches!(
            want.kind,
            SourceKind::Monitor | SourceKind::Dashboard | SourceKind::SLO
        ) {
            allowed.remove(SYNC_ID_KEY);
        }
        assert!(
            keys.is_subset(&allowed),
            "{id}: unexpected payload keys {keys:?}"
        );
        let hash = s.raw[CONTENT_HASH_KEY].as_str().unwrap_or_default();
        assert!(
            hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()),
            "{id}: {hash:?}"
        );
        assert_eq!(
            s.raw[CHUNK_COUNT_KEY],
            json!(chunk_counts[want.parent_id()]),
            "{id}"
        );

        // `Kind` is exactly the value the retrieval filter matches for that kind.
        let scope = RetrievalScope {
            kinds: vec![want.kind.clone()],
            ..Default::default()
        };
        let filter = scope.to_qdrant_filter().unwrap();
        assert_eq!(s.raw["Kind"], filter["must"][0]["match"]["any"][0], "{id}");
        // Timestamps are null or RFC 3339, which the datetime range filter parses.
        let ts = &s.raw["Timestamp"];
        assert!(
            ts.is_null() || ts.as_str().and_then(rag_core::planner::parse_utc).is_some(),
            "{id}: Timestamp {ts}"
        );
        // Logs are pattern day documents: the filter reads `Metadata.first_seen`, the
        // day's last log is the `Timestamp`, both on the document's UTC day, and the
        // reader parses the hourly counts it counts windows with.
        if want.kind == SourceKind::Logs {
            assert!(
                id.starts_with(rag_core::log_patterns::DOC_ID_PREFIX),
                "{id}"
            );
            let md = &s.raw["Metadata"];
            let first = md["first_seen"]
                .as_str()
                .and_then(rag_core::planner::parse_utc);
            let last = ts.as_str().and_then(rag_core::planner::parse_utc);
            assert!(first.is_some() && first <= last, "{id}: {md}");
            assert_eq!(md["last_seen"], *ts, "{id}");
            let day = md["day"].as_str().unwrap();
            for t in [first, last] {
                assert_eq!(t.unwrap().date_naive().to_string(), day, "{id}");
            }
            assert!(id.contains(&format!("_{day}#c")), "{id}");
            let state = rag_core::log_patterns::from_document(&s.doc).expect(id);
            assert_eq!(json!(state.count()), md["count"], "{id}");
            assert_eq!(state.pattern_id(), md["pattern_id"].as_str().unwrap());
        }
        // Service/Environment are stored in the form the scope matches.
        for key in ["Service", "Environment"] {
            let v = s.raw[key].as_str().unwrap();
            assert_eq!(v, v.trim().to_lowercase(), "{id}: {key} {v:?}");
        }
    }
}

/// Every point has exactly the dense and sparse vectors the hybrid query searches: a
/// `DIM`-sized dense vector and the sparse vector of the chunk's embedding input.
async fn check_stored_vectors(store: &Store, expected: &BTreeMap<String, RagDocument>) {
    let by_point: BTreeMap<String, &RagDocument> = expected
        .values()
        .map(|d| (point_id(&d.id).to_string(), d))
        .collect();
    let stored = store.vectors().await;
    assert_eq!(stored.len(), expected.len());
    for (pid, vectors) in stored {
        let doc = by_point[&pid];
        let names: BTreeSet<&str> = vectors
            .as_object()
            .unwrap_or_else(|| panic!("{}: unnamed vector {vectors}", doc.id))
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            names,
            BTreeSet::from([DENSE_VECTOR, SPARSE_VECTOR]),
            "{}",
            doc.id
        );
        assert_eq!(vectors[DENSE_VECTOR].as_array().unwrap().len(), DIM);
        let want = document_vector(&embedding_input(doc));
        assert!(!want.is_empty(), "{}", doc.id);
        let sparse = &vectors[SPARSE_VECTOR];
        assert_eq!(sparse["indices"], json!(want.indices), "{}", doc.id);
        let values: Vec<f32> = sparse["values"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap() as f32)
            .collect();
        assert_eq!(values, want.values, "{}", doc.id);
    }
}

/// Every source is a stored document, numbered like the answer prompt, and exposes
/// only the documented fields.
fn check_sources(resp: &Value, stored: &BTreeMap<String, support::StoredChunk>, prompt: &str) {
    let sources = resp["sources"].as_array().unwrap();
    let numbered = support::openai::prompt_numbers(prompt);
    assert_eq!(numbered.len(), sources.len(), "prompt documents vs sources");
    let mut seen = BTreeSet::new();
    let mut groups = BTreeSet::new();
    for (i, s) in sources.iter().enumerate() {
        let keys: BTreeSet<&str> = s.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            BTreeSet::from([
                "n",
                "id",
                "title",
                "kind",
                "timestamp",
                "service",
                "environment",
                "uri"
            ]),
            "source fields"
        );
        assert_eq!(s["n"], i + 1);
        let id = s["id"].as_str().unwrap();
        assert!(seen.insert(id.to_string()), "{id} listed twice in sources");
        let doc = &stored
            .values()
            .find(|c| c.doc.parent_id() == id)
            .unwrap_or_else(|| panic!("source {id} is not a stored document"))
            .doc;
        assert!(
            groups.insert(doc.group_id().to_string()),
            "{id}: its pattern is listed twice in sources"
        );
        assert_eq!(s["uri"], doc.source_uri);
        assert_eq!(s["title"], doc.title);
        assert_eq!(s["kind"], doc.kind.name());
        let (n, title, uri) = &numbered[i];
        assert_eq!(*n, i + 1);
        assert!(title.starts_with(&format!("{} (", doc.title)), "{title}");
        assert_eq!(uri, &doc.source_uri);
    }
}

/// The source a stored document is listed as ([`RagDocument::group_id`]).
fn group_of(stored: &BTreeMap<String, support::StoredChunk>, parent: &str) -> String {
    stored
        .values()
        .find(|c| c.doc.parent_id() == parent)
        .unwrap_or_else(|| panic!("{parent} not stored"))
        .doc
        .group_id()
        .to_string()
}

/// Source links of the stored documents listed as `group`.
fn uris_of(stored: &BTreeMap<String, support::StoredChunk>, group: &str) -> Vec<String> {
    stored
        .values()
        .filter(|c| c.doc.group_id() == group)
        .map(|c| c.doc.source_uri.clone())
        .collect()
}

struct Case {
    question: &'static str,
    /// Canned planner output.
    plan: Value,
    /// Extra request fields (explicit scope).
    request: Value,
    must: &'static [&'static str],
    must_not: &'static [&'static str],
    /// Every source must satisfy this.
    each: fn(&Value) -> bool,
    /// (log, text): the prompt block of the pattern holding the log contains the text.
    evidence: &'static [(&'static str, &'static str)],
}

fn cases() -> Vec<Case> {
    vec![
        // Datadog's `Auth-API` and the planner's `AUTH-API` meet as `auth-api`.
        Case {
            question: "Which auth-api incidents and errors were there yesterday?",
            plan: json!({"intent": "incidentSummary", "service": "AUTH-API", "environment": "Prod"}),
            request: json!({}),
            must: &["incident_inc-auth", "log_auth-1"],
            must_not: &[],
            each: |s| s["service"] == "auth-api" && s["environment"] == "prod",
            evidence: &[],
        },
        // Half-open window on RFC 3339 timestamps; timeless kinds still pass; two UTC
        // days of one pattern are one source, counted per local day.
        Case {
            question: "checkout errors yesterday",
            plan: json!({"intent": "semanticLogSearch", "service": "checkout", "environment": "prod"}),
            request: json!({}),
            must: &[
                "log_co-in",
                "log_co-span-1",
                "log_co-night-1",
                "monitor_7001",
                "slo_slo-checkout",
            ],
            must_not: &[
                "log_co-before",
                "log_co-at-end",
                "log_co-staging",
                "monitor_7002",
            ],
            each: |s| s["service"] == "checkout" && s["environment"] == "prod",
            evidence: &[(
                "log_co-night-1",
                "Occurrences in the question's window: 2 (Wed 2026-03-11: 2; days in Europe/Stockholm",
            )],
        },
        // A day whose first and last log are outside a short window but which logged
        // inside it is found through `Metadata.first_seen`; a day that logged only before
        // and after the window is dropped.
        Case {
            question: "checkout errors between 10 and 12",
            plan: json!({"intent": "semanticLogSearch", "service": "checkout", "environment": "prod"}),
            request: json!({"from_utc": "2026-03-11T10:00:00Z", "to_utc": "2026-03-11T12:00:00Z", "kinds": ["logs"]}),
            must: &["log_co-in", "log_co-span-1"],
            must_not: &["log_co-gap-1", "log_co-night-2", "log_co-before"],
            each: |s| s["kind"] == "logs",
            evidence: &[(
                "log_co-span-1",
                "Occurrences in the question's window: 1 (Wed 2026-03-11: 1; days in Europe/Stockholm",
            )],
        },
        // A dashboard's service comes from its widget queries (`Auth-API`) and meets
        // the planner's `AUTH-API`; the unscoped fixture dashboards stay out.
        Case {
            question: "Which dashboards show auth-api logins?",
            plan: json!({"intent": "dashboardLookup", "service": "AUTH-API", "environment": "PROD"}),
            request: json!({"kinds": ["dashboard"]}),
            must: &["dashboard_dash-auth"],
            must_not: &["dashboard_npw-6di-usv", "dashboard_448-ktj-ezs"],
            each: |s| s["kind"] == "dashboard" && s["service"] == "auth-api",
            evidence: &[],
        },
        // `Kind` = "sLO" on both sides.
        Case {
            question: "availability objectives",
            plan: json!({}),
            request: json!({"kinds": ["SLO"]}),
            must: &["slo_slo-checkout", "slo_c2ce7fb6030c5c0b8035d1ce94dec12c"],
            must_not: &[],
            each: |s| s["kind"] == "slo",
            evidence: &[],
        },
        // Explicit scope wins and is normalized like stored values.
        Case {
            question: "checkout latency",
            plan: json!({"service": "payments"}),
            request: json!({"service": "Checkout", "env": "STAGING"}),
            must: &["monitor_7002", "log_co-staging"],
            must_not: &["monitor_7001", "log_co-in"],
            each: |s| s["service"] == "checkout" && s["environment"] == "staging",
            evidence: &[],
        },
        // Metric catalog entries carry the indexing time and are timeless.
        Case {
            question: "which metrics were reported yesterday",
            plan: json!({"filters": ["kind:metrics"]}),
            request: json!({}),
            must: &["metric_checkout_orders_completed", "metric_system_cpu_idle"],
            must_not: &[],
            each: |s| s["kind"] == "metrics",
            evidence: &[],
        },
        // A pattern spanning several chunks is one source.
        Case {
            question: "payments bank gateway errors",
            plan: json!({"service": "payments", "environment": "prod"}),
            request: json!({}),
            must: &["log_long"],
            must_not: &[],
            each: |s| s["service"] == "payments",
            evidence: &[],
        },
    ]
}

async fn run_contract(store: Store) {
    let now = at(NOW);
    let fake = FakeOpenAi::default();
    let openai = fake.start().await;
    let oa = support::openai_client(&openai.uri());
    let corpus = corpus();

    support::index(&corpus, oa.clone(), &store, now).await;
    let expected = support::expected_chunks(&corpus, now).await;
    let id = |raw: &str| corpus.resolve(raw);
    assert!(
        expected
            .values()
            .filter(|d| d.parent_id() == id("log_long"))
            .count()
            >= 3,
        "the long log pattern must span at least three chunks"
    );
    // Logs of one pattern are one document with their count; different messages,
    // environments and services are not.
    assert_eq!(id("log_co-in"), id("log_co-in-2"));
    assert_eq!(id("log_co-span-1"), id("log_co-span-3"));
    // One document per UTC day, one pattern ID across days.
    assert_ne!(id("log_co-night-1"), id("log_co-night-2"));
    let pattern_id =
        |raw: &str| expected[&format!("{}#c0", id(raw))].metadata["pattern_id"].clone();
    assert_eq!(pattern_id("log_co-night-1"), pattern_id("log_co-night-2"));
    assert_eq!(id("log_long"), id("log_long-2"));
    let grouped = &expected[&format!("{}#c0", id("log_co-in"))];
    assert_eq!(grouped.metadata["count"], 2);
    assert_eq!(
        grouped.metadata["sample_log_ids"],
        json!(["co-in", "co-in-2"])
    );
    let spanning = &expected[&format!("{}#c0", id("log_co-span-1"))];
    assert_eq!(spanning.metadata["first_seen"], "2026-03-11T09:30:00.000Z");
    assert_eq!(
        spanning.timestamp.as_deref(),
        Some("2026-03-11T12:30:00.000Z")
    );
    for (a, b) in [
        ("log_co-in", "log_co-staging"),
        ("log_co-in", "log_co-before"),
    ] {
        assert_ne!(id(a), id(b));
    }
    // Incident details and dashboard definitions reach the stored documents.
    let auth = &expected["incident_inc-auth#c0"].text;
    assert!(
        auth.contains("Roterade signeringsnyckeln tillbaka 🔑")
            && auth.contains("Postmortem: Postmortem 901\nGrundorsak: nyckelrotation"),
        "{auth}"
    );
    let flaky = &expected["incident_inc-flaky#c0"];
    assert!(!flaky.text.contains("Timeline") && !flaky.text.contains("Postmortem"));
    let dash = &expected["dashboard_dash-auth#c0"];
    assert_eq!(
        (dash.service.as_str(), dash.environment.as_str()),
        ("auth-api", "prod")
    );
    assert!(
        dash.text
            .contains("- Misslyckade inloggningar: sum:auth.login.failures")
    );
    check_stored_points(&store, &expected).await;
    check_stored_vectors(&store, &expected).await;
    assert!(store.qdrant().check_collection().await.unwrap());
    let stored = support::stored_chunks(&store).await;

    let base = support::spawn_api(oa.clone(), &store, now, None).await;
    for case in cases() {
        {
            let mut script = fake.script.lock().unwrap();
            script.plans.insert(case.question.into(), case.plan.clone());
            // Cite every expected document; the negative control cites unknown ones.
            script.answers.insert(
                case.question.into(),
                support::openai::AnswerScript {
                    cite_uris: case
                        .must
                        .iter()
                        .map(|m| uris_of(&stored, &group_of(&stored, &id(m))))
                        .collect(),
                    cite_observations: vec![],
                    extra: "Also [DOC #42] and [obs-1].".into(),
                },
            );
        }
        let mut req = json!({"question": case.question, "timezone": TZ});
        req.as_object_mut()
            .unwrap()
            .extend(case.request.as_object().unwrap().clone());
        let resp = support::ask(&base, &req).await;
        // Sources as the groups they list (a log pattern's days are one source).
        let ids: Vec<String> = support::source_ids(&resp)
            .iter()
            .map(|i| group_of(&stored, i))
            .collect();
        let ctx = format!("{}: sources {ids:?}", case.question);

        let must: Vec<String> = case
            .must
            .iter()
            .map(|m| group_of(&stored, &id(m)))
            .collect();
        for (m, raw) in must.iter().zip(case.must) {
            assert!(ids.contains(m), "{ctx}: missing {raw} ({m})");
        }
        for raw in case.must_not {
            assert!(
                !ids.contains(&group_of(&stored, &id(raw))),
                "{ctx}: {raw} leaked through the filter"
            );
        }
        for s in resp["sources"].as_array().unwrap() {
            assert!((case.each)(s), "{ctx}: {s} is out of scope");
        }
        let prompt = fake.script.lock().unwrap().prompts[case.question].clone();
        check_sources(&resp, &stored, &prompt);
        let blocks = support::openai::prompt_blocks(&prompt);
        for (raw, text) in case.evidence {
            let group = group_of(&stored, &id(raw));
            let n = ids.iter().position(|g| *g == group).unwrap() + 1;
            let (_, block) = blocks.iter().find(|(b, _)| *b == n).unwrap();
            assert!(
                block.contains(text),
                "{ctx}: {raw} block lacks {text:?}:\n{block}"
            );
        }

        // Every intended citation resolves to the intended document; the unknown ones
        // are reported, not silently accepted.
        let answer = resp["answer"].as_str().unwrap();
        let report = rag_core::citations::validate_citations(answer, ids.len(), &[] as &[&str]);
        let cited: Vec<&str> = report
            .documents
            .iter()
            .map(|n| ids[n - 1].as_str())
            .collect();
        assert_eq!(cited.len(), must.len(), "{ctx}: {answer}");
        assert!(
            must.iter().all(|m| cited.contains(&m.as_str())),
            "{ctx}: cited {cited:?}"
        );
        assert_eq!(
            resp["citationWarnings"],
            json!([{"citation": "DOC #42", "reason": "unknown_document"},
                   {"citation": "obs-1", "reason": "unknown_observation"}]),
            "{ctx}"
        );
    }

    // A second run 15 minutes later finds every document unchanged (only metric catalog
    // entries, stamped with the run time, are rewritten) and writes nothing else.
    let before = support::stored_chunks(&store).await;
    let requests = support::index(&corpus, oa, &store, now + chrono::Duration::minutes(15)).await;
    let after = support::stored_chunks(&store).await;
    let definitions: Vec<_> = requests
        .iter()
        .filter(|p| p.starts_with("/api/v1/dashboard/"))
        .collect();
    assert!(
        definitions.is_empty(),
        "unchanged dashboards were fetched again: {definitions:?}"
    );
    assert_eq!(
        before.keys().collect::<Vec<_>>(),
        after.keys().collect::<Vec<_>>()
    );
    for (id, b) in &before {
        if b.doc.kind == SourceKind::Metrics {
            continue;
        }
        let strip = |v: &Value| {
            let mut v = v.clone();
            v.as_object_mut().unwrap().remove(SYNC_ID_KEY);
            v
        };
        assert_eq!(
            strip(&b.raw),
            strip(&after[id].raw),
            "{id} changed on re-index"
        );
    }
    store.finish().await;
}

#[tokio::test]
async fn pipeline_contract_in_memory() {
    run_contract(Store::fake(DIM).await).await;
}

/// The same contract against a real Qdrant: `QDRANT_TEST_ENDPOINT=http://localhost:6333
/// cargo test -p rag-indexer pipeline_contract_real_qdrant -- --ignored`.
#[tokio::test]
#[ignore = "requires QDRANT_TEST_ENDPOINT pointing at an isolated Qdrant server"]
async fn pipeline_contract_real_qdrant() {
    run_contract(Store::real(DIM).await).await;
}
