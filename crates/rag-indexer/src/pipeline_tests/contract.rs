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
//! - `Metadata.chunk_of` vs the reranker's grouping: a three-chunk log is one source;
//! - point IDs: every point ID is the UUIDv5 of its logical chunk ID;
//! - bookkeeping keys (`ContentHash`, `ChunkCount`, `SyncId`) are written but never
//!   reach `RagDocument` or `sources`; a second run leaves unchanged documents alone;
//! - `sources` point at stored documents, are numbered like the prompt, and every
//!   citation in the answer resolves (unknown ones are reported in `citationWarnings`).

use super::support::{self, Corpus, FakeOpenAi, Store, at, openai::DIM};
use rag_core::{
    domain::{RagDocument, SourceKind},
    qdrant::{CHUNK_COUNT_KEY, CONTENT_HASH_KEY, SYNC_ID_KEY, point_id},
    retrieval::RetrievalScope,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

const NOW: &str = "2026-03-12T09:00:00Z";
const TZ: &str = "Europe/Stockholm";

/// Long enough for three chunks (1800 chars, 200 overlap), with multibyte text at
/// every position.
fn long_message() -> String {
    let unit = "Återförsök mot bankgatewayen misslyckades 💳 決済エラー; e\u{0301}tat inconnu. ";
    unit.repeat(4000 / unit.chars().count() + 1)
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
        "incidents": [
            {"id": "inc-auth", "type": "incidents", "attributes": {
                "public_id": 901, "title": "Auth-API login failures",
                "created": "2026-03-11T14:00:00+00:00", "state": "resolved",
                "customer_impact_scope": "Users could not log in",
                "fields": {"severity": {"type": "dropdown", "value": "SEV-1"},
                           "services": {"type": "autocomplete", "value": ["Auth-API"]},
                           "env": {"type": "dropdown", "value": "PROD"}}}}
        ],
        "logs": [
            log("auth-1", "Auth-API", "prod", "2026-03-11T14:02:00.000Z", "login failed för åsa.öberg"),
            log("co-in", "checkout", "prod", "2026-03-11T10:15:30.123Z", "db connection pool exhausted"),
            log("co-before", "checkout", "prod", "2026-03-10T22:59:59.000Z", "db connection pool exhausted"),
            log("co-at-end", "checkout", "prod", "2026-03-11T23:00:00.000Z", "db connection pool exhausted"),
            log("co-staging", "checkout", "staging", "2026-03-11T10:20:00.000Z", "db connection pool exhausted"),
            log("long", "payments", "prod", "2026-03-11T12:00:00.000Z", &long_message()),
        ]
    }))
}

fn corpus() -> Corpus {
    let mut c = Corpus::fixtures();
    c.extend(crafted());
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
        // Service/Environment are stored in the form the scope matches.
        for key in ["Service", "Environment"] {
            let v = s.raw[key].as_str().unwrap();
            assert_eq!(v, v.trim().to_lowercase(), "{id}: {key} {v:?}");
        }
    }
}

/// Every source is a stored document, numbered like the answer prompt, and exposes
/// only the documented fields.
fn check_sources(resp: &Value, stored: &BTreeMap<String, support::StoredChunk>, prompt: &str) {
    let sources = resp["sources"].as_array().unwrap();
    let numbered = support::openai::prompt_numbers(prompt);
    assert_eq!(numbered.len(), sources.len(), "prompt documents vs sources");
    let mut seen = BTreeSet::new();
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
        assert_eq!(s["uri"], doc.source_uri);
        assert_eq!(s["title"], doc.title);
        assert_eq!(s["kind"], doc.kind.name());
        let (n, title, uri) = &numbered[i];
        assert_eq!(*n, i + 1);
        assert!(title.starts_with(&format!("{} (", doc.title)), "{title}");
        assert_eq!(uri, &doc.source_uri);
    }
}

fn uri_of(stored: &BTreeMap<String, support::StoredChunk>, parent: &str) -> String {
    stored
        .values()
        .find(|c| c.doc.parent_id() == parent)
        .unwrap_or_else(|| panic!("{parent} not stored"))
        .doc
        .source_uri
        .clone()
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
        },
        // Half-open window on RFC 3339 timestamps; timeless kinds still pass.
        Case {
            question: "checkout errors yesterday",
            plan: json!({"intent": "semanticLogSearch", "service": "checkout", "environment": "prod"}),
            request: json!({}),
            must: &["log_co-in", "monitor_7001", "slo_slo-checkout"],
            must_not: &[
                "log_co-before",
                "log_co-at-end",
                "log_co-staging",
                "monitor_7002",
            ],
            each: |s| s["service"] == "checkout" && s["environment"] == "prod",
        },
        // `Kind` = "sLO" on both sides.
        Case {
            question: "availability objectives",
            plan: json!({}),
            request: json!({"kinds": ["SLO"]}),
            must: &["slo_slo-checkout", "slo_c2ce7fb6030c5c0b8035d1ce94dec12c"],
            must_not: &[],
            each: |s| s["kind"] == "slo",
        },
        // Explicit scope wins and is normalized like stored values.
        Case {
            question: "checkout latency",
            plan: json!({"service": "payments"}),
            request: json!({"service": "Checkout", "env": "STAGING"}),
            must: &["monitor_7002", "log_co-staging"],
            must_not: &["monitor_7001", "log_co-in"],
            each: |s| s["service"] == "checkout" && s["environment"] == "staging",
        },
        // Metric catalog entries carry the indexing time and are timeless.
        Case {
            question: "which metrics were reported yesterday",
            plan: json!({"filters": ["kind:metrics"]}),
            request: json!({}),
            must: &["metric_checkout_orders_completed", "metric_system_cpu_idle"],
            must_not: &[],
            each: |s| s["kind"] == "metrics",
        },
        // Three chunks of one log are one source.
        Case {
            question: "payments bank gateway errors",
            plan: json!({"service": "payments", "environment": "prod"}),
            request: json!({}),
            must: &["log_long"],
            must_not: &[],
            each: |s| s["service"] == "payments",
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
    assert_eq!(
        expected
            .values()
            .filter(|d| d.parent_id() == "log_long")
            .count(),
        3,
        "the long log must span three chunks"
    );
    check_stored_points(&store, &expected).await;
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
                    cite_uris: case.must.iter().map(|id| uri_of(&stored, id)).collect(),
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
        let ids = support::source_ids(&resp);
        let ctx = format!("{}: sources {ids:?}", case.question);

        for id in case.must {
            assert!(ids.iter().any(|s| s == id), "{ctx}: missing {id}");
        }
        for id in case.must_not {
            assert!(
                !ids.iter().any(|s| s == id),
                "{ctx}: {id} leaked through the filter"
            );
        }
        for s in resp["sources"].as_array().unwrap() {
            assert!((case.each)(s), "{ctx}: {s} is out of scope");
        }
        let prompt = fake.script.lock().unwrap().prompts[case.question].clone();
        check_sources(&resp, &stored, &prompt);

        // Every intended citation resolves to the intended document; the unknown ones
        // are reported, not silently accepted.
        let answer = resp["answer"].as_str().unwrap();
        let report = rag_core::citations::validate_citations(answer, ids.len(), &[] as &[&str]);
        let cited: Vec<&str> = report
            .documents
            .iter()
            .map(|n| ids[n - 1].as_str())
            .collect();
        assert_eq!(cited.len(), case.must.len(), "{ctx}: {answer}");
        assert!(
            case.must.iter().all(|id| cited.contains(id)),
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
    support::index(&corpus, oa, &store, now + chrono::Duration::minutes(15)).await;
    let after = support::stored_chunks(&store).await;
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
