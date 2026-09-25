//! The incident question set (`crates/rag-indexer/tests/incident_questions/`): its
//! data model, and how one `/ask` response is scored against it.
//!
//! Shared by the in-process harness (`rag-indexer`'s `pipeline_tests::quality`, fake
//! embeddings) and the end-to-end run of the built binaries (`tails-e2e`, real
//! embeddings). Both score with [`score`] and aggregate with [`aggregate`], so their
//! numbers mean the same thing. See docs/DEVELOPMENT.md#incident-question-set.
//!
//! Metrics per question and in aggregate (known gaps excluded):
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
//!   `citationWarnings`;
//! - evidence accuracy: share of `evidence` entries met, i.e. the document appears as
//!   exactly one `[DOC #n]` block of the answer prompt and that block states the given
//!   facts (for example a log pattern's count in the question's window).
//!
//! Documents are compared as the sources they are listed as: the days of one log pattern
//! (`Metadata.pattern_id`) are one source, so `log_<id>` matches whichever day
//! represents the pattern.
//!
//! Hard checks on every question ([`Row::hard`]): `sources` are stored documents
//! numbered like the prompt, no source is listed twice, and `citationWarnings` equals
//! [`validate_citations`] on the answer.

use crate::datadog::{Corpus, Live};
use crate::openai::{prompt_blocks, prompt_numbers};
use chrono::{DateTime, Duration, Utc};
use rag_core::citations::validate_citations;
use rag_core::domain::RagDocument;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub fn at(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

/// `crates/rag-indexer/tests/incident_questions` in this checkout.
pub fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rag-indexer/tests/incident_questions")
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Dataset {
    pub version: u32,
    pub description: String,
    pub defaults: Defaults,
    pub thresholds: Thresholds,
    pub questions: Vec<Question>,
}

#[derive(Debug, Deserialize)]
pub struct Defaults {
    pub now: String,
    pub timezone: String,
}

#[derive(Debug, Clone, Copy, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Thresholds {
    pub recall_at_k: f64,
    pub precision_at_r: f64,
    pub distractor_exclusion: f64,
    pub scope_accuracy: f64,
    pub citation_validity: f64,
    pub citation_accuracy: f64,
    pub observation_recall: f64,
    pub negative_controls_flagged: f64,
    pub evidence_accuracy: f64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Question {
    pub id: String,
    pub question: String,
    pub note: Option<String>,
    pub known_gap: Option<String>,
    pub now: Option<String>,
    pub timezone: Option<String>,
    pub plan: Value,
    pub request: Option<Value>,
    pub expect: Expect,
    pub live: Option<LiveSpec>,
    pub hypotheses: Option<Value>,
    pub answer: AnswerSpec,
}

impl Question {
    /// The question's `now`, or the dataset default.
    pub fn now(&self, defaults: &Defaults) -> DateTime<Utc> {
        at(self.now.as_deref().unwrap_or(&defaults.now))
    }

    /// The question's timezone, or the dataset default.
    pub fn timezone<'a>(&'a self, defaults: &'a Defaults) -> &'a str {
        self.timezone.as_deref().unwrap_or(&defaults.timezone)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Expect {
    pub scope: Value,
    pub must_retrieve: Vec<String>,
    #[serde(default)]
    pub may_retrieve: Vec<String>,
    #[serde(default)]
    pub must_not_retrieve: Vec<String>,
    pub timeline: Option<TimelineExpect>,
    /// Facts the answer model must be given about a source (see [`EvidenceExpect`]).
    #[serde(default)]
    pub evidence: Vec<EvidenceExpect>,
}

/// What the prompt must tell the answer model about one source: `document` appears as
/// exactly one `[DOC #n]` block, and that block contains every string in `contains`
/// (for example the count of a log pattern in the question's window). This checks the
/// evidence text itself, which retrieval metrics cannot see.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceExpect {
    pub document: String,
    pub contains: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimelineExpect {
    pub status: String,
    #[serde(default)]
    pub observations: Vec<ObservationExpect>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationExpect {
    pub id: String,
    pub kind: String,
    pub service: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveSpec {
    #[serde(default)]
    pub metrics: Vec<MetricSpec>,
    #[serde(default)]
    pub logs: Vec<LogSpec>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricSpec {
    pub query: String,
    pub baseline: f64,
    pub spikes: Vec<Point>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Point {
    pub at: String,
    pub value: f64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogSpec {
    pub query: String,
    pub bursts: Vec<Burst>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Burst {
    pub at: String,
    pub count: usize,
    pub message: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AnswerSpec {
    #[serde(default)]
    pub cite_documents: Vec<String>,
    #[serde(default)]
    pub cite_observations: Vec<String>,
    #[serde(default)]
    pub extra: String,
    #[serde(default)]
    pub expect_citation_warnings: Vec<String>,
}

fn read_json(path: &Path) -> Value {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{path:?}: {e}"))
}

/// `questions.json` and the corpus it is asked over: `corpus.json` plus the recorded
/// fixtures in `fixtures`. Logs are indexed as pattern documents, so `log_<id>` in the
/// expectations is replaced by the ID of the pattern day document holding log `<id>`.
pub fn load(questions: &Path, corpus: &Path, fixtures: &Path) -> (Dataset, Corpus) {
    let mut dataset: Dataset =
        serde_json::from_value(read_json(questions)).expect("questions.json");
    let mut corpus = Corpus::from_json(&read_json(corpus));
    corpus.extend(Corpus::fixtures_from(fixtures));
    let resolve = |ids: &mut Vec<String>| {
        let mut out: Vec<String> = vec![];
        for id in ids.iter().map(|id| corpus.resolve(id)) {
            if !out.contains(&id) {
                out.push(id);
            }
        }
        *ids = out;
    };
    for q in &mut dataset.questions {
        resolve(&mut q.expect.must_retrieve);
        resolve(&mut q.expect.may_retrieve);
        resolve(&mut q.expect.must_not_retrieve);
        resolve(&mut q.answer.cite_documents);
        for x in &mut q.expect.evidence {
            x.document = corpus.resolve(&x.document);
        }
    }
    (dataset, corpus)
}

/// [`load`] from `dir` (`questions.json`, `corpus.json`) with the fixtures of this
/// checkout.
pub fn load_dir(dir: &Path) -> (Dataset, Corpus) {
    load(
        &dir.join("questions.json"),
        &dir.join("corpus.json"),
        &crate::datadog::fixture_dir(),
    )
}

/// Live Datadog data for one question: hourly series over the four days before
/// `now` (covering any window and its baseline) and bursts of error logs.
pub fn live_data(spec: &LiveSpec, now: DateTime<Utc>) -> Live {
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

/// The source each stored document is listed as: a log pattern's day documents share
/// `Metadata.pattern_id` and are one source; any other document is its own.
pub fn source_groups(parents: &BTreeMap<&str, &RagDocument>) -> BTreeMap<String, String> {
    parents
        .iter()
        .map(|(id, d)| {
            let group = d.metadata.get("pattern_id").and_then(Value::as_str);
            (id.to_string(), group.unwrap_or(id).to_string())
        })
        .collect()
}

/// Document IDs the questions name that are not among `parents`: a typo in the dataset
/// must fail loudly, not read as a retrieval miss.
pub fn unknown_ids(dataset: &Dataset, parents: &BTreeMap<&str, &RagDocument>) -> Vec<String> {
    let mut out = vec![];
    for q in &dataset.questions {
        let e = &q.expect;
        for id in e
            .must_retrieve
            .iter()
            .chain(&e.may_retrieve)
            .chain(&e.must_not_retrieve)
            .chain(&q.answer.cite_documents)
            .chain(e.evidence.iter().map(|x| &x.document))
        {
            if !parents.contains_key(id.as_str()) {
                out.push(format!("{}: unknown document {id}", q.id));
            }
        }
    }
    out
}

/// Rewrites every expectation from document IDs to the sources they are listed as
/// (see [`source_groups`]).
pub fn to_groups(dataset: &mut Dataset, groups: &BTreeMap<String, String>) {
    for q in &mut dataset.questions {
        let to_groups = |ids: &mut Vec<String>| {
            let mut out: Vec<String> = vec![];
            for g in ids.iter().map(|id| groups[id].clone()) {
                if !out.contains(&g) {
                    out.push(g);
                }
            }
            *ids = out;
        };
        to_groups(&mut q.expect.must_retrieve);
        to_groups(&mut q.expect.may_retrieve);
        to_groups(&mut q.expect.must_not_retrieve);
        to_groups(&mut q.answer.cite_documents);
        for x in &mut q.expect.evidence {
            x.document = groups[&x.document].clone();
        }
    }
}

/// For each document the answer of `q` cites (as a source group), the source URIs it
/// may appear under in the prompt.
pub fn cite_uris(
    q: &Question,
    parents: &BTreeMap<&str, &RagDocument>,
    groups: &BTreeMap<String, String>,
) -> Vec<Vec<String>> {
    q.answer
        .cite_documents
        .iter()
        .map(|g| {
            parents
                .iter()
                .filter(|(id, _)| groups[**id] == *g)
                .map(|(_, d)| d.source_uri.clone())
                .collect()
        })
        .collect()
}

/// Which answer model produced the responses being scored.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Answers {
    /// The canned answer model of [`crate::openai::FakeOpenAi`]: its prompt is known,
    /// so numbering, evidence, citation accuracy and negative controls are checked.
    Canned,
    /// A real chat model: only what holds for any answer is checked.
    Real,
}

/// The scores of one question, and what went wrong.
#[derive(Default, Debug)]
pub struct Row {
    pub id: String,
    pub known_gap: bool,
    pub recall: f64,
    pub precision: f64,
    pub excluded: usize,
    pub distractors: usize,
    pub scope_ok: bool,
    pub citations_valid: usize,
    pub citations_total: usize,
    pub intended_ok: usize,
    pub intended_total: usize,
    pub obs_found: usize,
    pub obs_expected: usize,
    pub negatives_flagged: usize,
    pub negatives_total: usize,
    pub evidence_ok: usize,
    pub evidence_total: usize,
    pub sources: usize,
    /// Metric misses, reported in the notes.
    pub problems: Vec<String>,
    /// Broken invariants that must hold whatever the embeddings: a source that is not
    /// stored, listed twice or numbered unlike the prompt, or `citationWarnings`
    /// disagreeing with `validate_citations`.
    pub hard: Vec<String>,
}

fn ratio(n: usize, d: usize) -> f64 {
    if d == 0 { 1.0 } else { n as f64 / d as f64 }
}

pub struct Aggregate {
    pub recall_at_k: f64,
    pub precision_at_r: f64,
    pub distractor_exclusion: f64,
    pub scope_accuracy: f64,
    pub citation_validity: f64,
    pub citation_accuracy: f64,
    pub observation_recall: f64,
    pub negative_controls_flagged: f64,
    pub evidence_accuracy: f64,
}

pub fn aggregate(rows: &[Row]) -> Aggregate {
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
        evidence_accuracy: ratio(sum(|r| r.evidence_ok), sum(|r| r.evidence_total)),
    }
}

/// IDs of `sources` in `[DOC #n]` order.
pub fn source_ids(resp: &Value) -> Vec<String> {
    resp["sources"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|s| s["id"].as_str().unwrap_or_default().to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// Scores one response and checks the invariants that must always hold.
pub fn score(
    q: &Question,
    resp: &Value,
    prompt: Option<&str>,
    parents: &BTreeMap<&str, &RagDocument>,
    groups: &BTreeMap<String, String>,
    answers: Answers,
) -> Row {
    let e = &q.expect;
    // Sources as the groups the expectations name (see `source_groups`).
    let ids: Vec<String> = source_ids(resp)
        .iter()
        .map(|id| groups.get(id).cloned().unwrap_or_else(|| id.clone()))
        .collect();
    let empty = vec![];
    let sources = resp["sources"].as_array().unwrap_or(&empty);
    let mut row = Row {
        id: q.id.clone(),
        known_gap: q.known_gap.is_some(),
        sources: ids.len(),
        ..Row::default()
    };
    if !resp["sources"].is_array() {
        row.hard.push(format!("no sources array in {resp}"));
    }

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
    for (k, want) in e.scope.as_object().into_iter().flatten() {
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
        let id = s["id"].as_str().unwrap_or_default();
        let Some(doc) = parents.get(id) else {
            row.hard.push(format!("source {id} is not stored"));
            continue;
        };
        if s["uri"] != doc.source_uri.as_str() {
            row.hard.push(format!(
                "{id}: uri {} != stored {}",
                s["uri"], doc.source_uri
            ));
        }
        if s["title"] != doc.title.as_str() {
            row.hard.push(format!(
                "{id}: title {} != stored {}",
                s["title"], doc.title
            ));
        }
    }
    for (i, id) in ids.iter().enumerate() {
        if ids[..i].contains(id) {
            row.hard.push(format!("{id} is listed twice in sources"));
        }
    }
    for (i, s) in sources.iter().enumerate() {
        if s["n"] != i + 1 {
            row.hard.push(format!(
                "sources[{i}] is numbered {} (want {})",
                s["n"],
                i + 1
            ));
        }
    }
    let prompt = prompt.filter(|_| answers == Answers::Canned);
    if let Some(prompt) = prompt {
        let numbered = prompt_numbers(prompt);
        if numbered.len() != sources.len() {
            row.hard.push(format!(
                "the prompt has {} documents, sources {}",
                numbered.len(),
                sources.len()
            ));
        }
        for ((n, title, uri), s) in numbered.iter().zip(sources) {
            let title_ok = s["title"]
                .as_str()
                .is_some_and(|t| title.starts_with(&format!("{t} (")));
            if s["n"] != *n || s["uri"] != uri.as_str() || !title_ok {
                row.hard.push(format!(
                    "prompt [DOC #{n}] {title} {uri} is not source {} {} {}",
                    s["n"], s["title"], s["uri"]
                ));
            }
        }
    } else if answers == Answers::Canned && !sources.is_empty() {
        row.hard
            .push("no answer prompt was recorded for a question with sources".into());
    }

    // Evidence given to the answer model.
    if answers == Answers::Canned {
        let blocks = prompt_blocks(prompt.unwrap_or_default());
        for x in &e.evidence {
            row.evidence_total += 1;
            let mine: Vec<&String> = blocks
                .iter()
                .filter(|(n, _)| ids.get(n.wrapping_sub(1)) == Some(&x.document))
                .map(|(_, b)| b)
                .collect();
            let [block] = mine.as_slice() else {
                row.problems.push(format!(
                    "{} is in {} prompt documents, want 1",
                    x.document,
                    mine.len()
                ));
                continue;
            };
            let missing: Vec<&String> = x.contains.iter().filter(|c| !block.contains(*c)).collect();
            if missing.is_empty() {
                row.evidence_ok += 1;
            } else {
                let facts: Vec<&str> = block
                    .lines()
                    .filter(|l| l.starts_with("Occurrences") || l.starts_with("Excerpt"))
                    .collect();
                row.problems.push(format!(
                    "{} evidence lacks {missing:?} (has {facts:?})",
                    x.document
                ));
            }
        }
    }

    // Citations.
    let answer = resp["answer"].as_str().unwrap_or_default();
    let report = validate_citations(answer, ids.len(), &obs_ids);
    let expected_warnings = serde_json::to_value(&report.warnings).unwrap();
    if resp["citationWarnings"] != expected_warnings {
        row.hard.push(format!(
            "citationWarnings {} disagree with validate_citations {expected_warnings}",
            resp["citationWarnings"]
        ));
    }
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
    if answers == Answers::Canned {
        row.negatives_total = negatives.len();
        row.negatives_flagged = negatives
            .iter()
            .filter(|n| warned.contains(&n.as_str()))
            .count();
        // Intended document citations must point at the intended document.
        let cited: Vec<&str> = report
            .documents
            .iter()
            .filter_map(|n| ids.get(n.wrapping_sub(1)).map(String::as_str))
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

/// The report's column header.
pub fn print_header() {
    println!(
        "{:<28} {:>6} {:>6} {:>7} {:>5} {:>7} {:>7} {:>5} {:>5} {:>4}  notes",
        "question",
        "recall",
        "prec@R",
        "exclude",
        "scope",
        "cites",
        "intent",
        "obs",
        "evid",
        "srcs"
    );
}

/// One report line.
pub fn print_row(row: &Row) {
    let mut notes = row.problems.clone();
    notes.extend(row.hard.iter().map(|h| format!("HARD: {h}")));
    println!(
        "{:<28} {:>6.2} {:>6.2} {:>7} {:>5} {:>7} {:>7} {:>5} {:>5} {:>4}  {}{}",
        row.id.chars().take(28).collect::<String>(),
        row.recall,
        row.precision,
        format!("{}/{}", row.excluded, row.distractors),
        if row.scope_ok { "ok" } else { "FAIL" },
        format!("{}/{}", row.citations_valid, row.citations_total),
        format!("{}/{}", row.intended_ok, row.intended_total),
        format!("{}/{}", row.obs_found, row.obs_expected),
        format!("{}/{}", row.evidence_ok, row.evidence_total),
        row.sources,
        if row.known_gap { "KNOWN GAP; " } else { "" },
        notes.join("; "),
    );
}

fn metrics(a: &Aggregate, t: &Thresholds) -> [(&'static str, f64, f64); 9] {
    [
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
        (
            "evidence accuracy",
            a.evidence_accuracy,
            t.evidence_accuracy,
        ),
    ]
}

/// The aggregate lines of the report, and which known gaps now pass.
pub fn print_aggregate(rows: &[Row], t: &Thresholds) {
    let a = aggregate(rows);
    println!(
        "aggregate over {} questions (known gaps excluded):",
        rows.iter().filter(|r| !r.known_gap).count()
    );
    for (name, got, min) in metrics(&a, t) {
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
}

/// Aggregates below their threshold. Citation accuracy, negative controls and evidence
/// accuracy need the canned answer model's prompt and are only checked with it.
pub fn threshold_failures(rows: &[Row], t: &Thresholds, answers: Answers) -> Vec<String> {
    let a = aggregate(rows);
    metrics(&a, t)
        .into_iter()
        .filter(|(name, _, _)| {
            answers == Answers::Canned
                || !matches!(
                    *name,
                    "citation accuracy" | "negative controls flagged" | "evidence accuracy"
                )
        })
        .filter(|(_, got, min)| got + 1e-9 < *min)
        .map(|(name, got, min)| format!("{name} {got:.3} < {min:.2}"))
        .collect()
}
