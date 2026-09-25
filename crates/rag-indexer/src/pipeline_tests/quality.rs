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
//! The data model, the metrics and the hard checks on every question are in
//! [`tails_fakes::questions`], shared with the end-to-end run of the built binaries
//! (`tails-e2e`, real embeddings).
//!
//! The test fails when a hard check breaks or an aggregate drops below `thresholds` in
//! `questions.json`. Run with `-- --nocapture` for the report. See docs/DEVELOPMENT.md.

use super::support::{self, FakeOpenAi, Store, openai::DIM};
use rag_core::openai::OpenAiClient;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use tails_fakes::questions::{self, Answers, Row, Thresholds};

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

impl Models {
    fn answers(self) -> Answers {
        match self {
            Models::Fake => Answers::Canned,
            Models::OpenAi => Answers::Real,
        }
    }
}

async fn run(store: Store, models: Models) -> (Vec<Row>, Thresholds) {
    let (mut dataset, corpus) = questions::load_dir(&questions::data_dir());
    let fake = FakeOpenAi::default();
    let fake_server = fake.start().await;
    let oa: OpenAiClient = match models {
        Models::Fake => support::openai_client(&fake_server.uri()),
        Models::OpenAi => OpenAiClient::new_from_env().expect("OPENAI_API_KEY"),
    };
    let index_now = support::at(&dataset.defaults.now);
    support::index(&corpus, oa.clone(), &store, index_now).await;
    let stored = support::stored_chunks(&store).await;
    let parents: BTreeMap<&str, &rag_core::domain::RagDocument> = stored
        .values()
        .map(|c| (c.doc.parent_id(), &c.doc))
        .collect();

    // A typo in the dataset must fail loudly, not read as a retrieval miss.
    let unknown = questions::unknown_ids(&dataset, &parents);
    assert!(unknown.is_empty(), "{unknown:?}");
    // Expectations name sources: the days of one log pattern are one source.
    let groups = questions::source_groups(&parents);
    questions::to_groups(&mut dataset, &groups);

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
    questions::print_header();

    let mut rows = vec![];
    for q in &dataset.questions {
        let now = q.now(&dataset.defaults);
        let tz = q.timezone(&dataset.defaults);
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
                    cite_uris: questions::cite_uris(q, &parents, &groups),
                    cite_observations: q.answer.cite_observations.clone(),
                    extra: q.answer.extra.clone(),
                },
            );
        }
        let live = support::datadog::serve_live(questions::live_data(
            q.live.as_ref().unwrap_or(&questions::LiveSpec::default()),
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
        let prompt = fake.prompt(&q.question);
        let row = questions::score(
            q,
            &resp,
            prompt.as_deref(),
            &parents,
            &groups,
            models.answers(),
        );
        questions::print_row(&row);
        assert!(row.hard.is_empty(), "{}: {:?}", q.id, row.hard);
        rows.push(row);
    }
    let t = dataset.thresholds;
    questions::print_aggregate(&rows, &t);
    store.finish().await;
    (rows, t)
}

fn assert_thresholds(rows: &[Row], t: Thresholds, models: Models) {
    let failures = questions::threshold_failures(rows, &t, models.answers());
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
