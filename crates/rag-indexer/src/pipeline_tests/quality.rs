//! The incident question set: answer quality of the whole pipeline, measured.
//!
//! `tests/incident_questions/questions.json` lists incident questions with a fixed
//! "now" and timezone, the scope the planner should produce, the documents that must
//! (and must not) be retrieved, the live observations expected in the timeline, and
//! what the answer should cite. The corpus (`corpus.json` plus the recorded fixtures)
//! is indexed with the real indexer; each question then goes through the real `/ask`
//! with a canned planner reply, deterministic embeddings and a canned answer model
//! that cites by reading its prompt, so every run is identical.
//!
//! Per question and in aggregate (known gaps excluded) it measures:
//! - `recall@k`: share of `mustRetrieve` documents among `sources` (k = all sources
//!   given to the answer model, i.e. the reranked top-K);
//! - `precision@R`: share of the first R sources that are `mustRetrieve` or
//!   `mayRetrieve`, with R = number of `mustRetrieve` documents (ranking quality);
//! - distractor exclusion: share of `mustNotRetrieve` documents kept out of `sources`;
//! - scope accuracy: the response's `scope` equals the expected one;
//! - citation validity: share of citations in answers that resolve to a source or an
//!   observation (deliberate negative controls excluded);
//! - citation accuracy: share of intended citations that point at the intended
//!   document, i.e. `[DOC #n]` in the answer is `sources[n-1]`;
//! - observation recall: expected timeline observations present with kind and service;
//! - negative controls flagged: deliberately unknown citations reported in
//!   `citationWarnings`.
//!
//! Hard checks on every question: `sources` are stored documents numbered like the
//! prompt, and `citationWarnings` equals [`validate_citations`] on the answer.
//!
//! The test fails when an aggregate drops below `thresholds` in `questions.json`. Run
//! with `-- --nocapture` for the report. See docs/DEVELOPMENT.md.

use super::support::{self, Corpus, FakeOpenAi, Store, at, datadog::Live, openai::DIM};
use chrono::{DateTime, Duration, Utc};
use rag_core::citations::validate_citations;
use rag_core::openai::OpenAiClient;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::PathBuf;

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/incident_questions")
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Dataset {
    version: u32,
    #[allow(dead_code)]
    description: String,
    defaults: Defaults,
    thresholds: Thresholds,
    questions: Vec<Question>,
}

#[derive(Debug, Deserialize)]
struct Defaults {
    now: String,
    timezone: String,
}

#[derive(Debug, Clone, Copy, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Thresholds {
    recall_at_k: f64,
    precision_at_r: f64,
    distractor_exclusion: f64,
    scope_accuracy: f64,
    citation_validity: f64,
    citation_accuracy: f64,
    observation_recall: f64,
    negative_controls_flagged: f64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Question {
    id: String,
    question: String,
    #[allow(dead_code)]
    note: Option<String>,
    known_gap: Option<String>,
    now: Option<String>,
    timezone: Option<String>,
    plan: Value,
    request: Option<Value>,
    expect: Expect,
    live: Option<LiveSpec>,
    hypotheses: Option<Value>,
    answer: AnswerSpec,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Expect {
    scope: Value,
    must_retrieve: Vec<String>,
    #[serde(default)]
    may_retrieve: Vec<String>,
    #[serde(default)]
    must_not_retrieve: Vec<String>,
    timeline: Option<TimelineExpect>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TimelineExpect {
    status: String,
    #[serde(default)]
    observations: Vec<ObservationExpect>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ObservationExpect {
    id: String,
    kind: String,
    service: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct LiveSpec {
    #[serde(default)]
    metrics: Vec<MetricSpec>,
    #[serde(default)]
    logs: Vec<LogSpec>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MetricSpec {
    query: String,
    baseline: f64,
    spikes: Vec<Point>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Point {
    at: String,
    value: f64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LogSpec {
    query: String,
    bursts: Vec<Burst>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Burst {
    at: String,
    count: usize,
    message: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AnswerSpec {
    #[serde(default)]
    cite_documents: Vec<String>,
    #[serde(default)]
    cite_observations: Vec<String>,
    #[serde(default)]
    extra: String,
    #[serde(default)]
    expect_citation_warnings: Vec<String>,
}

fn load() -> (Dataset, Corpus) {
    let read = |name: &str| -> Value {
        let path = data_dir().join(name);
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap())
            .unwrap_or_else(|e| panic!("{path:?}: {e}"))
    };
    let dataset: Dataset = serde_json::from_value(read("questions.json")).expect("questions.json");
    let mut corpus = Corpus::from_json(&read("corpus.json"));
    corpus.extend(Corpus::fixtures());
    (dataset, corpus)
}

/// Live Datadog data for one question: hourly series over the four days before
/// `now` (covering any window and its baseline) and bursts of error logs.
fn live_data(spec: &LiveSpec, now: DateTime<Utc>) -> Live {
    let mut live = Live::default();
    for m in &spec.metrics {
        let spikes: Vec<(DateTime<Utc>, f64)> =
            m.spikes.iter().map(|p| (at(&p.at), p.value)).collect();
        let from = now - Duration::days(4);
        let from = from - Duration::seconds(from.timestamp().rem_euclid(3600));
        live.series.insert(
            m.query.clone(),
            Live::hourly(from, now, m.baseline, &spikes),
        );
    }
    for l in &spec.logs {
        let tag = |key: &str| {
            l.query
                .split_whitespace()
                .find_map(|t| t.strip_prefix(key))
                .unwrap_or_default()
                .to_string()
        };
        let (service, env) = (tag("service:"), tag("env:"));
        let events: Vec<Value> = l
            .bursts
            .iter()
            .flat_map(|b| {
                let start = at(&b.at);
                (0..b.count)
                    .map(move |i| (start + Duration::seconds(30 * i as i64), b.message.clone()))
            })
            .enumerate()
            .map(|(i, (t, message))| {
                json!({"id": format!("live-{service}-{i}"), "type": "log", "attributes": {
                    "service": service, "status": "error", "message": message,
                    "timestamp": t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                    "tags": [format!("env:{env}")]
                }})
            })
            .collect();
        live.logs.insert(l.query.clone(), events);
    }
    live
}

/// Which answer model and embeddings the run uses.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Models {
    /// Deterministic fakes; the default, and what CI runs.
    Fake,
    /// `OPENAI_API_KEY` (and optional `OPENAI_BASE_URL`, `OPENAI_EMBEDDING_MODEL`,
    /// `OPENAI_CHAT_MODEL`): real embeddings and answers, canned plans sent with the
    /// request. Citation accuracy and negative controls are not measured.
    OpenAi,
}

#[derive(Default)]
struct Row {
    id: String,
    known_gap: bool,
    recall: f64,
    precision: f64,
    excluded: usize,
    distractors: usize,
    scope_ok: bool,
    citations_valid: usize,
    citations_total: usize,
    intended_ok: usize,
    intended_total: usize,
    obs_found: usize,
    obs_expected: usize,
    negatives_flagged: usize,
    negatives_total: usize,
    sources: usize,
    problems: Vec<String>,
}

fn ratio(n: usize, d: usize) -> f64 {
    if d == 0 { 1.0 } else { n as f64 / d as f64 }
}

struct Aggregate {
    recall_at_k: f64,
    precision_at_r: f64,
    distractor_exclusion: f64,
    scope_accuracy: f64,
    citation_validity: f64,
    citation_accuracy: f64,
    observation_recall: f64,
    negative_controls_flagged: f64,
}

fn aggregate(rows: &[Row]) -> Aggregate {
    let counted: Vec<&Row> = rows.iter().filter(|r| !r.known_gap).collect();
    let n = counted.len().max(1) as f64;
    let sum = |f: fn(&Row) -> usize| counted.iter().map(|r| f(r)).sum::<usize>();
    Aggregate {
        recall_at_k: counted.iter().map(|r| r.recall).sum::<f64>() / n,
        precision_at_r: counted.iter().map(|r| r.precision).sum::<f64>() / n,
        distractor_exclusion: ratio(sum(|r| r.excluded), sum(|r| r.distractors)),
        scope_accuracy: counted.iter().filter(|r| r.scope_ok).count() as f64 / n,
        citation_validity: ratio(sum(|r| r.citations_valid), sum(|r| r.citations_total)),
        citation_accuracy: ratio(sum(|r| r.intended_ok), sum(|r| r.intended_total)),
        observation_recall: ratio(sum(|r| r.obs_found), sum(|r| r.obs_expected)),
        negative_controls_flagged: ratio(sum(|r| r.negatives_flagged), sum(|r| r.negatives_total)),
    }
}

async fn run(store: Store, models: Models) -> (Vec<Row>, Thresholds) {
    let (dataset, corpus) = load();
    let fake = FakeOpenAi::default();
    let fake_server = fake.start().await;
    let oa: OpenAiClient = match models {
        Models::Fake => support::openai_client(&fake_server.uri()),
        Models::OpenAi => OpenAiClient::new_from_env().expect("OPENAI_API_KEY"),
    };
    let index_now = at(&dataset.defaults.now);
    support::index(&corpus, oa.clone(), &store, index_now).await;
    let stored = support::stored_chunks(&store).await;
    let parents: BTreeMap<&str, &rag_core::domain::RagDocument> = stored
        .values()
        .map(|c| (c.doc.parent_id(), &c.doc))
        .collect();

    // A typo in the dataset must fail loudly, not read as a retrieval miss.
    for q in &dataset.questions {
        let e = &q.expect;
        for id in e
            .must_retrieve
            .iter()
            .chain(&e.may_retrieve)
            .chain(&e.must_not_retrieve)
            .chain(&q.answer.cite_documents)
        {
            assert!(
                parents.contains_key(id.as_str()),
                "{}: unknown document {id}",
                q.id
            );
        }
    }

    println!(
        "\nincident questions v{} | store: {} | models: {}",
        dataset.version,
        store.describe(),
        if models == Models::Fake {
            "fake embeddings + canned answers"
        } else {
            "OpenAI"
        }
    );
    println!(
        "{:<28} {:>6} {:>6} {:>7} {:>5} {:>7} {:>7} {:>5} {:>4}  notes",
        "question", "recall", "prec@R", "exclude", "scope", "cites", "intent", "obs", "srcs"
    );

    let mut rows = vec![];
    for q in &dataset.questions {
        let now = at(q.now.as_deref().unwrap_or(&dataset.defaults.now));
        let tz = q.timezone.as_deref().unwrap_or(&dataset.defaults.timezone);
        {
            let mut script = fake.script.lock().unwrap();
            script.plans.insert(q.question.clone(), q.plan.clone());
            if let Some(h) = &q.hypotheses {
                script
                    .hypotheses
                    .insert(q.question.clone(), json!({"hypotheses": h}));
            }
            script.answers.insert(
                q.question.clone(),
                support::openai::AnswerScript {
                    cite_uris: q
                        .answer
                        .cite_documents
                        .iter()
                        .map(|id| parents[id.as_str()].source_uri.clone())
                        .collect(),
                    cite_observations: q.answer.cite_observations.clone(),
                    extra: q.answer.extra.clone(),
                },
            );
        }
        let live = support::datadog::serve_live(live_data(
            q.live.as_ref().unwrap_or(&LiveSpec::default()),
            now,
        ))
        .await;
        let base = support::spawn_api(oa.clone(), &store, now, Some(&live)).await;
        let mut req = json!({"question": q.question, "timezone": tz});
        if let Some(extra) = q.request.as_ref().and_then(Value::as_object) {
            req.as_object_mut().unwrap().extend(extra.clone());
        }
        if models == Models::OpenAi {
            req["plan"] = q.plan.clone();
        }
        let resp = support::ask(&base, &req).await;
        let prompt = fake
            .script
            .lock()
            .unwrap()
            .prompts
            .get(&q.question)
            .cloned();
        let row = score(q, &resp, prompt.as_deref(), &parents, models);
        println!(
            "{:<28} {:>6.2} {:>6.2} {:>7} {:>5} {:>7} {:>7} {:>5} {:>4}  {}{}",
            q.id.chars().take(28).collect::<String>(),
            row.recall,
            row.precision,
            format!("{}/{}", row.excluded, row.distractors),
            if row.scope_ok { "ok" } else { "FAIL" },
            format!("{}/{}", row.citations_valid, row.citations_total),
            format!("{}/{}", row.intended_ok, row.intended_total),
            format!("{}/{}", row.obs_found, row.obs_expected),
            row.sources,
            if row.known_gap { "KNOWN GAP; " } else { "" },
            row.problems.join("; "),
        );
        rows.push(row);
    }
    let a = aggregate(&rows);
    let t = dataset.thresholds;
    println!(
        "aggregate over {} questions (known gaps excluded):",
        rows.iter().filter(|r| !r.known_gap).count()
    );
    for (name, got, min) in [
        ("recall@k", a.recall_at_k, t.recall_at_k),
        ("precision@R", a.precision_at_r, t.precision_at_r),
        (
            "distractor exclusion",
            a.distractor_exclusion,
            t.distractor_exclusion,
        ),
        ("scope accuracy", a.scope_accuracy, t.scope_accuracy),
        (
            "citation validity",
            a.citation_validity,
            t.citation_validity,
        ),
        (
            "citation accuracy",
            a.citation_accuracy,
            t.citation_accuracy,
        ),
        (
            "observation recall",
            a.observation_recall,
            t.observation_recall,
        ),
        (
            "negative controls flagged",
            a.negative_controls_flagged,
            t.negative_controls_flagged,
        ),
    ] {
        println!(
            "  {name:<26} {got:.3} (threshold {min:.2}){}",
            if got < min { "  BELOW THRESHOLD" } else { "" }
        );
    }
    for r in rows.iter().filter(|r| r.known_gap) {
        let passes = r.problems.is_empty();
        println!(
            "  known gap {}: {}",
            r.id,
            if passes {
                "now passes; remove `knownGap` so it counts"
            } else {
                "still failing"
            }
        );
    }
    store.finish().await;
    (rows, t)
}

/// Scores one response and checks the invariants that must always hold.
fn score(
    q: &Question,
    resp: &Value,
    prompt: Option<&str>,
    parents: &BTreeMap<&str, &rag_core::domain::RagDocument>,
    models: Models,
) -> Row {
    let e = &q.expect;
    let ids = support::source_ids(resp);
    let sources = resp["sources"].as_array().unwrap();
    let mut row = Row {
        id: q.id.clone(),
        known_gap: q.known_gap.is_some(),
        sources: ids.len(),
        ..Row::default()
    };

    // Retrieval.
    let found: Vec<&String> = e
        .must_retrieve
        .iter()
        .filter(|id| ids.contains(id))
        .collect();
    row.recall = ratio(found.len(), e.must_retrieve.len());
    let r = e.must_retrieve.len().min(ids.len());
    let relevant = |id: &String| e.must_retrieve.contains(id) || e.may_retrieve.contains(id);
    row.precision = if r == 0 {
        0.0
    } else {
        ids[..r].iter().filter(|id| relevant(id)).count() as f64 / r as f64
    };
    let leaked: Vec<&String> = e
        .must_not_retrieve
        .iter()
        .filter(|id| ids.contains(id))
        .collect();
    row.distractors = e.must_not_retrieve.len();
    row.excluded = row.distractors - leaked.len();
    if found.len() < e.must_retrieve.len() {
        let missed: Vec<&String> = e
            .must_retrieve
            .iter()
            .filter(|id| !ids.contains(id))
            .collect();
        row.problems.push(format!("missed {missed:?}"));
    }
    if !leaked.is_empty() {
        row.problems.push(format!("distractors {leaked:?}"));
    }
    if row.precision < 1.0 {
        row.problems
            .push(format!("top {:?}", &ids[..r.max(1).min(ids.len())]));
    }

    // Scope: only the keys the dataset states.
    for (k, want) in e.scope.as_object().unwrap() {
        if resp["scope"][k] != *want {
            row.problems
                .push(format!("scope.{k} = {} (want {want})", resp["scope"][k]));
        }
    }
    row.scope_ok = !row.problems.iter().any(|p| p.starts_with("scope."));

    // Timeline.
    let observations = resp["timeline"]["observations"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let obs_ids: Vec<&str> = observations
        .iter()
        .filter_map(|o| o["id"].as_str())
        .collect();
    if let Some(t) = &e.timeline {
        if resp["timeline"]["status"] != t.status.as_str() {
            row.problems
                .push(format!("timeline.status = {}", resp["timeline"]["status"]));
        }
        row.obs_expected = t.observations.len();
        for o in &t.observations {
            let hit = observations.iter().any(|x| {
                x["id"] == o.id.as_str()
                    && x["kind"] == o.kind.as_str()
                    && o.service
                        .as_ref()
                        .is_none_or(|s| x["service"] == s.as_str())
            });
            if hit {
                row.obs_found += 1;
            } else {
                row.problems
                    .push(format!("no {} {} {:?}", o.id, o.kind, o.service));
            }
        }
        if row.obs_found < row.obs_expected {
            let got: Vec<String> = observations
                .iter()
                .map(|o| format!("{} {} {}", o["id"], o["kind"], o["service"]))
                .collect();
            let missing: Vec<String> = resp["timeline"]["missingEvidence"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|m| format!("{} {}", m["subject"], m["reason"]))
                .collect();
            row.problems
                .push(format!("observed {got:?}, missing {missing:?}"));
        }
    }

    // Hard invariants: sources are stored documents, numbered like the prompt.
    for s in sources {
        let id = s["id"].as_str().unwrap();
        let doc = parents
            .get(id)
            .unwrap_or_else(|| panic!("{}: source {id} is not stored", q.id));
        assert_eq!(s["uri"], doc.source_uri, "{}: {id}", q.id);
        assert_eq!(s["title"], doc.title, "{}: {id}", q.id);
    }
    if let Some(prompt) = prompt.filter(|_| models == Models::Fake) {
        let numbered = support::openai::prompt_numbers(prompt);
        assert_eq!(numbered.len(), sources.len(), "{}: prompt vs sources", q.id);
        for ((n, title, uri), s) in numbered.iter().zip(sources) {
            assert_eq!(s["n"], *n, "{}", q.id);
            assert_eq!(s["uri"], uri.as_str(), "{}", q.id);
            assert!(
                title.starts_with(&format!("{} (", s["title"].as_str().unwrap())),
                "{}",
                q.id
            );
        }
    }

    // Citations.
    let answer = resp["answer"].as_str().unwrap_or_default();
    let report = validate_citations(answer, ids.len(), &obs_ids);
    assert_eq!(
        resp["citationWarnings"],
        serde_json::to_value(&report.warnings).unwrap(),
        "{}: citationWarnings disagree with validate_citations",
        q.id
    );
    let warned: Vec<&str> = report
        .warnings
        .iter()
        .map(|w| w.citation.as_str())
        .collect();
    let negatives = &q.answer.expect_citation_warnings;
    let unexpected = warned
        .iter()
        .filter(|w| !negatives.iter().any(|n| n == *w))
        .count();
    row.citations_valid = report.cited();
    row.citations_total = report.cited() + unexpected;
    if unexpected > 0 {
        row.problems
            .push(format!("unresolved citations {warned:?}"));
    }
    if models == Models::Fake {
        row.negatives_total = negatives.len();
        row.negatives_flagged = negatives
            .iter()
            .filter(|n| warned.contains(&n.as_str()))
            .count();
        // Intended document citations must point at the intended document.
        let cited: Vec<&str> = report
            .documents
            .iter()
            .map(|n| ids[n - 1].as_str())
            .collect();
        for id in &q.answer.cite_documents {
            if !ids.contains(id) {
                continue; // a retrieval miss, already counted in recall
            }
            row.intended_total += 1;
            if cited.contains(&id.as_str()) {
                row.intended_ok += 1;
            } else {
                row.problems.push(format!("{id} not cited"));
            }
        }
        let unintended: Vec<&&str> = cited
            .iter()
            .filter(|c| !q.answer.cite_documents.iter().any(|d| d == *c))
            .collect();
        if !unintended.is_empty() {
            row.intended_total += unintended.len();
            row.problems
                .push(format!("cited unintended {unintended:?}"));
        }
        for o in &q.answer.cite_observations {
            row.intended_total += 1;
            if report.observations.iter().any(|x| x == o) {
                row.intended_ok += 1;
            }
        }
    }
    row
}

fn assert_thresholds(rows: &[Row], t: Thresholds, models: Models) {
    let a = aggregate(rows);
    let mut failures = vec![];
    let mut check = |name: &str, got: f64, min: f64| {
        if got + 1e-9 < min {
            failures.push(format!("{name} {got:.3} < {min:.2}"));
        }
    };
    check("recall@k", a.recall_at_k, t.recall_at_k);
    check("precision@R", a.precision_at_r, t.precision_at_r);
    check(
        "distractor exclusion",
        a.distractor_exclusion,
        t.distractor_exclusion,
    );
    check("scope accuracy", a.scope_accuracy, t.scope_accuracy);
    check(
        "citation validity",
        a.citation_validity,
        t.citation_validity,
    );
    check(
        "observation recall",
        a.observation_recall,
        t.observation_recall,
    );
    if models == Models::Fake {
        check(
            "citation accuracy",
            a.citation_accuracy,
            t.citation_accuracy,
        );
        check(
            "negative controls flagged",
            a.negative_controls_flagged,
            t.negative_controls_flagged,
        );
    }
    assert!(
        failures.is_empty(),
        "incident questions below threshold (run with -- --nocapture for the report): {failures:?}"
    );
}

#[tokio::test]
async fn incident_questions_in_memory() {
    let (rows, t) = run(Store::fake(DIM).await, Models::Fake).await;
    assert_thresholds(&rows, t, Models::Fake);
}

/// `QDRANT_TEST_ENDPOINT=http://localhost:6333 cargo test -p rag-indexer
/// incident_questions_real_qdrant -- --ignored --nocapture`
#[tokio::test]
#[ignore = "requires QDRANT_TEST_ENDPOINT pointing at an isolated Qdrant server"]
async fn incident_questions_real_qdrant() {
    let (rows, t) = run(Store::real(DIM).await, Models::Fake).await;
    assert_thresholds(&rows, t, Models::Fake);
}

/// Real embeddings and answers; never run in CI. Uses the real Qdrant when
/// `QDRANT_TEST_ENDPOINT` is set, otherwise the in-memory store:
/// `OPENAI_API_KEY=... cargo test -p rag-indexer incident_questions_openai -- --ignored --nocapture`
#[tokio::test]
#[ignore = "calls the OpenAI API (OPENAI_API_KEY); opt-in only"]
async fn incident_questions_openai() {
    let oa = OpenAiClient::new_from_env().expect("OPENAI_API_KEY");
    let dim = oa.embed("dimension probe").await.expect("embedding").len();
    let store = if std::env::var("QDRANT_TEST_ENDPOINT").is_ok() {
        Store::real(dim).await
    } else {
        Store::fake(dim).await
    };
    let (rows, t) = run(store, Models::OpenAi).await;
    assert_thresholds(&rows, t, Models::OpenAi);
}
