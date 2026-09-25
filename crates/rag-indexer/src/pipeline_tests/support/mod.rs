//! Shared harness: run the real indexer into a store, serve the real API router on
//! the same store, and read back what was written.

pub use tails_fakes::{datadog, openai};
pub mod qdrant;

use crate::checkpoint::{self, Checkpoints, Window};
use crate::incremental::{IncrementalSink, IndexParams};
use crate::{
    CHUNK_OVERLAP, CHUNK_SIZE, EMBED_BATCH_MAX_CHARS, IndexerConfig, Source, SourceFetcher,
    dedupe_by_id, index_sources,
};
use chrono::{DateTime, Duration, Utc};
use rag_api::{AppState, Limits};
use rag_core::{
    chunk::chunk, domain::RagDocument, live_evidence::LiveEvidenceConfig, openai::OpenAiClient,
    planner::FixedClock, qdrant::QdrantPayload, rag_service::StageTimeouts,
    resilience::RetryPolicy,
};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;
use wiremock::MockServer;

pub use datadog::Corpus;
pub use openai::FakeOpenAi;
pub use qdrant::Store;

pub use tails_fakes::questions::at;

/// Every source's first run reaches back far enough for all fixtures and the corpus.
fn config() -> IndexerConfig {
    IndexerConfig {
        lookback: Duration::days(5 * 365),
        overlap: Duration::minutes(10),
        disabled: vec![],
    }
}

/// The OpenAI client both the indexer and the API use, pointed at `uri`.
pub fn openai_client(uri: &str) -> OpenAiClient {
    let mut oa = OpenAiClient::new(
        "sk-test".into(),
        uri.into(),
        "fake-bow-1024".into(),
        "fake-chat".into(),
    );
    oa.retry = RetryPolicy::none();
    oa
}

/// The production write path: fetch every source from the (fake) Datadog API, dedupe,
/// then `IncrementalSink` (chunk, hash, batch-embed, upsert, clean up) into `store`,
/// exactly as `main` wires it, with small embedding batches to exercise batching.
/// Returns the paths of the Datadog requests the run made.
pub async fn index(
    corpus: &Corpus,
    oa: OpenAiClient,
    store: &Store,
    now: DateTime<Utc>,
) -> Vec<String> {
    let (server, dd) = datadog::serve(corpus.clone()).await;
    let sink = IncrementalSink {
        embedder: oa,
        store: store.qdrant(),
        params: IndexParams {
            chunk_size: CHUNK_SIZE,
            chunk_overlap: CHUNK_OVERLAP,
            embed_batch_size: 8,
            embed_batch_max_chars: EMBED_BATCH_MAX_CHARS,
            concurrency: 4,
            allow_empty_sync_delete: false,
        },
    };
    let path = std::env::temp_dir().join(format!(
        "tails-pipeline-{}-{}.json",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    let mut checkpoints = Checkpoints::default();
    let failures = index_sources(&dd, &sink, &mut checkpoints, &path, &config(), now).await;
    let _ = std::fs::remove_file(&path);
    let failures: Vec<String> = failures
        .iter()
        .map(|(s, e)| format!("{}: {e:#}", s.name()))
        .collect();
    assert!(failures.is_empty(), "indexing failed: {failures:?}");
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .map(|r| r.url.path().to_string())
        .collect()
}

/// The chunks the indexer should have written for `corpus`: the adapters' documents
/// for the first-run window, deduplicated and chunked like production.
pub async fn expected_chunks(corpus: &Corpus, now: DateTime<Utc>) -> BTreeMap<String, RagDocument> {
    let (_server, dd) = datadog::serve(corpus.clone()).await;
    let c = config();
    let window: Window = checkpoint::window(None, now, c.lookback, c.overlap);
    let mut out = BTreeMap::new();
    for source in Source::ALL {
        let docs = dedupe_by_id(dd.fetch(source, &window).await.unwrap());
        for doc in &docs {
            for ch in chunk(CHUNK_SIZE, CHUNK_OVERLAP, doc) {
                out.insert(ch.id.clone(), ch);
            }
        }
    }
    out
}

/// A stored point decoded by the reader's own payload type.
pub struct StoredChunk {
    pub point_id: String,
    pub raw: Value,
    pub doc: RagDocument,
}

pub async fn stored_chunks(store: &Store) -> BTreeMap<String, StoredChunk> {
    let mut out = BTreeMap::new();
    for (point_id, raw) in store.points().await {
        let payload: QdrantPayload = serde_json::from_value(raw.clone())
            .unwrap_or_else(|e| panic!("the reader cannot decode stored payload {raw}: {e}"));
        let doc = RagDocument::from(payload);
        out.insert(doc.id.clone(), StoredChunk { point_id, raw, doc });
    }
    out
}

/// Serves the real API router on an ephemeral port: OpenAI at `oa`, retrieval from
/// `store`, a fixed clock, and live evidence from `live` when given.
pub async fn spawn_api(
    oa: OpenAiClient,
    store: &Store,
    now: DateTime<Utc>,
    live: Option<&MockServer>,
) -> String {
    let state = AppState {
        oa,
        qd: store.qdrant(),
        limits: Limits {
            ask_deadline: std::time::Duration::from_secs(60),
            stages: StageTimeouts {
                live_evidence: std::time::Duration::from_secs(10),
                ..StageTimeouts::default()
            },
        },
        clock: Arc::new(FixedClock(now)),
        dd: live.map(|s| Arc::new(datadog::client(s))),
        live: LiveEvidenceConfig::default(),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum_serve(listener, state).await });
    format!("http://{addr}")
}

async fn axum_serve(listener: tokio::net::TcpListener, state: AppState) {
    rag_api::serve(listener, state).await.unwrap();
}

/// POSTs `/ask` and returns the 200 response body.
pub async fn ask(base: &str, body: &Value) -> Value {
    let r = reqwest::Client::new()
        .post(format!("{base}/ask"))
        .json(body)
        .send()
        .await
        .unwrap();
    let status = r.status();
    let v: Value = r.json().await.unwrap();
    assert!(status.is_success(), "/ask {body} -> {status}: {v}");
    v
}

/// IDs of `sources` in `[DOC #n]` order.
pub fn source_ids(resp: &Value) -> Vec<String> {
    resp["sources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap().to_string())
        .collect()
}
