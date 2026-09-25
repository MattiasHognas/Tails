mod checkpoint;
mod incremental;
#[cfg(test)]
mod pipeline_tests;

use anyhow::Result;
use checkpoint::{Checkpoints, Window};
use chrono::{DateTime, Duration, Utc};
use incremental::{
    Embedder, IncrementalSink, IndexParams, IndexStats, Metadata, PointStore, SyncScope,
};
use rag_core::{
    datadog::Datadog,
    domain::{RagDocument, SourceKind},
    log_patterns,
    openai::OpenAiClient,
    qdrant::Qdrant,
};
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Maximum characters per chunk
const CHUNK_SIZE: usize = 1800;
/// Characters of overlap between consecutive chunks
const CHUNK_OVERLAP: usize = 200;
/// Default texts per embeddings request (OpenAI accepts up to 2048)
const EMBED_BATCH_SIZE: usize = 128;
/// Default characters per embeddings request, well under OpenAI's per-request token limit
const EMBED_BATCH_MAX_CHARS: usize = 200_000;
/// Default embedding batches, lookups and deletes in flight
const EMBED_CONCURRENCY: usize = 4;
/// Upper bound for `INDEXER_EMBED_CONCURRENCY`
const MAX_EMBED_CONCURRENCY: usize = 32;

/// A Datadog source, indexed and checkpointed independently of the others.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Monitors,
    Dashboards,
    Slos,
    Metrics,
    Incidents,
    Logs,
}

impl Source {
    const ALL: [Source; 6] = [
        Source::Monitors,
        Source::Dashboards,
        Source::Slos,
        Source::Metrics,
        Source::Incidents,
        Source::Logs,
    ];

    /// Key of the source in the checkpoint file.
    fn name(self) -> &'static str {
        match self {
            Source::Monitors => "monitors",
            Source::Dashboards => "dashboards",
            Source::Slos => "slos",
            Source::Metrics => "metrics",
            Source::Incidents => "incidents",
            Source::Logs => "logs",
        }
    }

    fn names() -> Vec<&'static str> {
        Self::ALL.iter().map(|s| s.name()).collect()
    }

    /// Monitors, dashboards and SLOs are fetched in full, so documents missing from a
    /// successful fetch were deleted in Datadog. The other sources are windowed.
    fn sync_scope(self, sync_id: &str) -> SyncScope {
        let kind = match self {
            Source::Monitors => SourceKind::Monitor,
            Source::Dashboards => SourceKind::Dashboard,
            Source::Slos => SourceKind::SLO,
            Source::Metrics | Source::Incidents | Source::Logs => return SyncScope::Window,
        };
        SyncScope::Full {
            kind,
            sync_id: sync_id.to_string(),
        }
    }
}

/// Fetches the documents of one source for a window.
trait SourceFetcher {
    async fn fetch(&self, source: Source, window: &Window) -> Result<Vec<RagDocument>>;
}

/// Stores the documents of one fetch and removes what they replace. Returns only after
/// every write (upserts and deletes) succeeded.
trait DocumentSink {
    async fn index(&self, docs: &[RagDocument], scope: &SyncScope) -> Result<IndexStats>;
    /// Stored metadata of the documents in `doc_ids` that are indexed.
    async fn stored_metadata(&self, doc_ids: &[String]) -> Result<HashMap<String, Metadata>>;
}

impl SourceFetcher for Datadog {
    async fn fetch(&self, source: Source, window: &Window) -> Result<Vec<RagDocument>> {
        let from_iso = window.from.to_rfc3339();
        let to_iso = window.to.to_rfc3339();
        match source {
            // Configuration objects are small and re-synced in full on every run.
            Source::Monitors => self.get_monitors().await,
            Source::Dashboards => self.list_dashboards().await,
            Source::Slos => self.list_slos().await,
            Source::Metrics => self.list_metrics(&from_iso, &to_iso).await,
            Source::Incidents => self.get_incidents(&from_iso, &to_iso).await,
            // From a whole minute, so pattern counts line up across runs.
            Source::Logs => {
                let from = log_patterns::fetch_start(window.from).to_rfc3339();
                self.search_logs(&from, &to_iso).await
            }
        }
    }
}

impl<E: Embedder, S: PointStore> DocumentSink for IncrementalSink<E, S> {
    async fn index(&self, docs: &[RagDocument], scope: &SyncScope) -> Result<IndexStats> {
        IncrementalSink::index(self, docs, scope).await
    }

    async fn stored_metadata(&self, doc_ids: &[String]) -> Result<HashMap<String, Metadata>> {
        IncrementalSink::stored_metadata(self, doc_ids).await
    }
}

/// Log pattern documents count only the logs of this run's fetch. Combines each with
/// its stored point, so the stored count, first and last time seen cover every run and
/// logs in the overlap with the previous run are counted once (see
/// [`rag_core::log_patterns`]).
async fn merge_log_patterns(
    sink: &impl DocumentSink,
    docs: Vec<RagDocument>,
    window: &Window,
) -> Result<Vec<RagDocument>> {
    let ids: Vec<String> = docs.iter().map(|d| d.id.clone()).collect();
    let stored = sink.stored_metadata(&ids).await?;
    let start = log_patterns::fetch_start(window.from);
    Ok(docs
        .into_iter()
        .map(|d| match stored.get(&d.id) {
            Some(md) => log_patterns::merge_document(d, md, start),
            None => d,
        })
        .collect())
}

/// Drops documents whose id was already seen, keeping the first occurrence. Overlapping
/// windows and shifting pagination can return the same record twice within a run; across
/// runs, deterministic point IDs make re-upserting a document overwrite it.
fn dedupe_by_id(docs: Vec<RagDocument>) -> Vec<RagDocument> {
    let mut seen = HashSet::new();
    docs.into_iter()
        .filter(|d| seen.insert(d.id.clone()))
        .collect()
}

struct IndexerConfig {
    lookback: Duration,
    overlap: Duration,
}

/// Indexes every source, advancing and persisting each source's checkpoint only after
/// its documents are stored and obsolete points removed. A failing source is logged and
/// skipped so the others still advance; the failures are returned.
async fn index_sources(
    fetcher: &impl SourceFetcher,
    sink: &impl DocumentSink,
    checkpoints: &mut Checkpoints,
    checkpoint_path: &Path,
    config: &IndexerConfig,
    now: DateTime<Utc>,
) -> Vec<(Source, anyhow::Error)> {
    let mut failures = Vec::new();
    // Marks the points of full-sync documents seen in this run.
    let sync_id = now.to_rfc3339();
    for source in Source::ALL {
        let window = checkpoint::window(
            checkpoints.get(source.name()),
            now,
            config.lookback,
            config.overlap,
        );
        tracing::info!(
            "Indexing {} from {} to {}",
            source.name(),
            window.from.to_rfc3339(),
            window.to.to_rfc3339()
        );

        // A failed fetch never reaches the sink, so it can't delete anything.
        let result = async {
            let mut docs = dedupe_by_id(fetcher.fetch(source, &window).await?);
            if source == Source::Logs {
                docs = merge_log_patterns(sink, docs, &window).await?;
            }
            sink.index(&docs, &source.sync_scope(&sync_id)).await
        }
        .await;

        match result {
            Ok(stats) => {
                tracing::info!(
                    "Indexed {}: {} fetched, {} unchanged, {} embedded ({} chunks), \
                     {} shrunk, {} stale points deleted",
                    source.name(),
                    stats.fetched,
                    stats.unchanged,
                    stats.embedded_docs,
                    stats.embedded_chunks,
                    stats.shrunk,
                    stats.deleted_stale
                );
                checkpoints.set(source.name(), window.to);
                if let Err(e) = checkpoints.save(checkpoint_path).await {
                    tracing::error!("Failed to save {} checkpoint: {:#}", source.name(), e);
                    failures.push((source, e));
                }
            }
            Err(e) => {
                tracing::error!("Failed to index {}: {:#}", source.name(), e);
                failures.push((source, e));
            }
        }
    }
    failures
}

fn env_minutes(name: &str, default: i64) -> i64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(default)
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(default)
}

fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .map(|v| {
            matches!(
                v.trim().to_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

/// Incremental indexing settings from the environment, clamped so batches are never
/// empty and concurrency is never unbounded.
fn index_params_from_env() -> IndexParams {
    IndexParams {
        chunk_size: CHUNK_SIZE,
        chunk_overlap: CHUNK_OVERLAP,
        embed_batch_size: env_usize("INDEXER_EMBED_BATCH_SIZE", EMBED_BATCH_SIZE).clamp(1, 2048),
        embed_batch_max_chars: env_usize("INDEXER_EMBED_BATCH_MAX_CHARS", EMBED_BATCH_MAX_CHARS)
            .max(1),
        concurrency: env_usize("INDEXER_EMBED_CONCURRENCY", EMBED_CONCURRENCY)
            .clamp(1, MAX_EMBED_CONCURRENCY),
        allow_empty_sync_delete: env_flag("INDEXER_ALLOW_EMPTY_SYNC_DELETE"),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let checkpoint_path =
        std::env::var("INDEXER_WATERMARK").unwrap_or_else(|_| "./watermark.json".to_string());
    let config = IndexerConfig {
        lookback: Duration::minutes(env_minutes("INDEXER_LOOKBACK_MINUTES", 90)),
        overlap: Duration::minutes(env_minutes("INDEXER_OVERLAP_MINUTES", 10)),
    };

    let dd = Datadog::new_from_env()?;
    let sink = IncrementalSink {
        embedder: OpenAiClient::new_from_env()?,
        store: Qdrant::new_from_env()?,
        params: index_params_from_env(),
    };

    let checkpoint_path = Path::new(&checkpoint_path);
    let mut checkpoints = Checkpoints::load(checkpoint_path, &Source::names()).await?;

    let failures = index_sources(
        &dd,
        &sink,
        &mut checkpoints,
        checkpoint_path,
        &config,
        Utc::now(),
    )
    .await;

    if !failures.is_empty() {
        let names: Vec<_> = failures.iter().map(|(s, _)| s.name()).collect();
        anyhow::bail!("Indexing failed for: {}", names.join(", "));
    }
    tracing::info!("Indexing complete");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use incremental::fakes::{FakeEmbedder, FakeStore};
    use rag_core::{chunk::chunk, qdrant::QPoint};
    use std::sync::Mutex;

    fn chunk_documents(docs: &[RagDocument]) -> Vec<RagDocument> {
        docs.iter()
            .flat_map(|d| chunk(CHUNK_SIZE, CHUNK_OVERLAP, d))
            .collect()
    }

    fn create_test_doc() -> RagDocument {
        RagDocument {
            id: "test-id".to_string(),
            title: "Test Document".to_string(),
            text: "This is test content".to_string(),
            source_uri: "http://example.com/test".to_string(),
            kind: SourceKind::Monitor,
            timestamp: Some("2025-01-01T00:00:00Z".to_string()),
            service: "test-service".to_string(),
            environment: "production".to_string(),
            metadata: serde_json::Map::new(),
        }
    }

    #[test]
    fn test_payload_from_basic() {
        let doc = create_test_doc();
        let payload = serde_json::to_value(QPoint::from_document(&doc, vec![1.0]).payload).unwrap();

        assert_eq!(payload["id"], doc.id);
        assert_eq!(payload["Title"], "Test Document");
        assert_eq!(payload["Text"], "This is test content");
        assert_eq!(payload["SourceUri"], "http://example.com/test");
        assert_eq!(payload["Service"], "test-service");
        assert_eq!(payload["Environment"], "production");
    }

    #[test]
    fn test_payload_from_with_timestamp() {
        let doc = create_test_doc();
        let payload = serde_json::to_value(QPoint::from_document(&doc, vec![1.0]).payload).unwrap();

        assert_eq!(payload["Timestamp"], "2025-01-01T00:00:00Z");
    }

    #[test]
    fn test_payload_from_without_timestamp() {
        let mut doc = create_test_doc();
        doc.timestamp = None;
        let payload = serde_json::to_value(QPoint::from_document(&doc, vec![1.0]).payload).unwrap();

        assert!(payload["Timestamp"].is_null());
    }

    #[test]
    fn test_payload_from_preserves_kind() {
        let mut doc = create_test_doc();
        doc.kind = SourceKind::Incident;
        let payload = serde_json::to_value(QPoint::from_document(&doc, vec![1.0]).payload).unwrap();

        assert_eq!(payload["Kind"], serde_json::json!(SourceKind::Incident));
    }

    #[test]
    fn test_payload_from_with_metadata() {
        let mut doc = create_test_doc();
        let mut metadata = serde_json::Map::new();
        metadata.insert("severity".to_string(), serde_json::json!("SEV-1"));
        metadata.insert("status".to_string(), serde_json::json!("active"));
        doc.metadata = metadata;

        let payload = serde_json::to_value(QPoint::from_document(&doc, vec![1.0]).payload).unwrap();

        assert_eq!(payload["Metadata"]["severity"], "SEV-1");
        assert_eq!(payload["Metadata"]["status"], "active");
    }

    #[test]
    fn test_payload_from_empty_metadata() {
        let doc = create_test_doc();
        let payload = serde_json::to_value(QPoint::from_document(&doc, vec![1.0]).payload).unwrap();

        assert!(payload["Metadata"].is_object());
        assert_eq!(payload["Metadata"].as_object().unwrap().len(), 0);
    }

    #[test]
    fn test_payload_from_all_source_kinds() {
        let kinds = vec![
            SourceKind::Monitor,
            SourceKind::Incident,
            SourceKind::Logs,
            SourceKind::Dashboard,
            SourceKind::Metrics,
            SourceKind::SLO,
            SourceKind::Git,
        ];

        for kind in kinds {
            let mut doc = create_test_doc();
            doc.kind = kind.clone();
            let payload =
                serde_json::to_value(QPoint::from_document(&doc, vec![1.0]).payload).unwrap();

            assert_eq!(payload["Kind"], serde_json::json!(kind));
        }
    }

    /// Serves canned documents per source; sources in `failing` return an error.
    struct FakeFetcher {
        docs: HashMap<&'static str, Vec<RagDocument>>,
        failing: Vec<Source>,
        windows: Mutex<Vec<(Source, Window)>>,
    }

    impl FakeFetcher {
        fn new() -> Self {
            Self {
                docs: HashMap::new(),
                failing: Vec::new(),
                windows: Mutex::new(Vec::new()),
            }
        }

        fn window_of(&self, source: Source) -> Window {
            self.windows
                .lock()
                .unwrap()
                .iter()
                .rev()
                .find(|(s, _)| *s == source)
                .unwrap()
                .1
        }
    }

    impl SourceFetcher for FakeFetcher {
        async fn fetch(&self, source: Source, window: &Window) -> Result<Vec<RagDocument>> {
            self.windows.lock().unwrap().push((source, *window));
            if self.failing.contains(&source) {
                anyhow::bail!("{} unavailable", source.name());
            }
            Ok(self.docs.get(source.name()).cloned().unwrap_or_default())
        }
    }

    /// Stands in for Qdrant: points keyed by ID, so an upsert of an existing ID
    /// overwrites it. Fails when asked to index a document of `failing_kind`.
    #[derive(Default)]
    struct FakeSink {
        points: Mutex<HashMap<String, String>>,
        indexed_ids: Mutex<Vec<String>>,
        failing_kind: Option<SourceKind>,
    }

    impl DocumentSink for FakeSink {
        async fn index(&self, docs: &[RagDocument], _scope: &SyncScope) -> Result<IndexStats> {
            if docs
                .iter()
                .any(|d| Some(&d.kind) == self.failing_kind.as_ref())
            {
                anyhow::bail!("embedding failed");
            }
            self.indexed_ids
                .lock()
                .unwrap()
                .extend(docs.iter().map(|d| d.id.clone()));
            let chunks = chunk_documents(docs);
            let mut points = self.points.lock().unwrap();
            for c in &chunks {
                let point = QPoint::from_document(c, vec![0.0]);
                points.insert(point.id.to_string(), c.id.clone());
            }
            Ok(IndexStats {
                fetched: docs.len(),
                embedded_docs: docs.len(),
                embedded_chunks: chunks.len(),
                ..IndexStats::default()
            })
        }

        async fn stored_metadata(&self, _: &[String]) -> Result<HashMap<String, Metadata>> {
            Ok(HashMap::new())
        }
    }

    fn doc(id: &str, kind: SourceKind) -> RagDocument {
        RagDocument {
            id: id.to_string(),
            kind,
            ..create_test_doc()
        }
    }

    fn ts(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn temp_checkpoint(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("rag-indexer-main-{}-{}", std::process::id(), name));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("checkpoints.json");
        let _ = std::fs::remove_file(&path);
        path
    }

    fn config() -> IndexerConfig {
        IndexerConfig {
            lookback: Duration::minutes(90),
            overlap: Duration::minutes(10),
        }
    }

    #[test]
    fn test_dedupe_by_id_keeps_first_occurrence() {
        let mut second = doc("log_a", SourceKind::Logs);
        second.text = "duplicate".to_string();
        let docs = vec![
            doc("log_a", SourceKind::Logs),
            doc("log_b", SourceKind::Logs),
            second,
        ];

        let deduped = dedupe_by_id(docs);

        let ids: Vec<_> = deduped.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, ["log_a", "log_b"]);
        assert_eq!(deduped[0].text, "This is test content");
    }

    #[test]
    fn test_source_names_are_unique() {
        let names = Source::names();
        let unique: HashSet<_> = names.iter().collect();
        assert_eq!(unique.len(), Source::ALL.len());
    }

    #[tokio::test]
    async fn test_index_sources_advances_all_checkpoints() {
        let path = temp_checkpoint("advance");
        let mut fetcher = FakeFetcher::new();
        fetcher
            .docs
            .insert("logs", vec![doc("log_a", SourceKind::Logs)]);
        let sink = FakeSink::default();
        let mut checkpoints = Checkpoints::default();
        let now = ts("2025-01-01T12:00:00Z");

        let failures =
            index_sources(&fetcher, &sink, &mut checkpoints, &path, &config(), now).await;

        assert!(failures.is_empty());
        for source in Source::ALL {
            assert_eq!(checkpoints.get(source.name()), Some(now));
            // First run: lookback window.
            assert_eq!(fetcher.window_of(source).from, ts("2025-01-01T10:30:00Z"));
        }
        let saved = Checkpoints::load(&path, &Source::names()).await.unwrap();
        assert_eq!(saved, checkpoints);
    }

    #[tokio::test]
    async fn test_failing_fetch_does_not_block_other_sources() {
        let path = temp_checkpoint("fetch-failure");
        let before = ts("2025-01-01T11:45:00Z");
        let now = ts("2025-01-01T12:00:00Z");
        let mut checkpoints = Checkpoints::default();
        for name in Source::names() {
            checkpoints.set(name, before);
        }
        let mut fetcher = FakeFetcher::new();
        fetcher.failing = vec![Source::Incidents];
        fetcher
            .docs
            .insert("logs", vec![doc("log_a", SourceKind::Logs)]);
        let sink = FakeSink::default();

        let failures =
            index_sources(&fetcher, &sink, &mut checkpoints, &path, &config(), now).await;

        let failed: Vec<_> = failures.iter().map(|(s, _)| *s).collect();
        assert_eq!(failed, [Source::Incidents]);
        // Logs come after incidents and were still indexed.
        assert_eq!(*sink.indexed_ids.lock().unwrap(), ["log_a"]);

        let saved = Checkpoints::load(&path, &Source::names()).await.unwrap();
        assert_eq!(saved.get("incidents"), Some(before));
        for source in Source::ALL.into_iter().filter(|s| *s != Source::Incidents) {
            assert_eq!(saved.get(source.name()), Some(now), "{}", source.name());
        }
    }

    #[tokio::test]
    async fn test_failing_sink_does_not_advance_that_source() {
        let path = temp_checkpoint("sink-failure");
        let now = ts("2025-01-01T12:00:00Z");
        let mut fetcher = FakeFetcher::new();
        fetcher
            .docs
            .insert("logs", vec![doc("log_a", SourceKind::Logs)]);
        fetcher
            .docs
            .insert("incidents", vec![doc("incident_a", SourceKind::Incident)]);
        let sink = FakeSink {
            failing_kind: Some(SourceKind::Logs),
            ..FakeSink::default()
        };
        let mut checkpoints = Checkpoints::default();

        let failures =
            index_sources(&fetcher, &sink, &mut checkpoints, &path, &config(), now).await;

        let failed: Vec<_> = failures.iter().map(|(s, _)| *s).collect();
        assert_eq!(failed, [Source::Logs]);
        let saved = Checkpoints::load(&path, &Source::names()).await.unwrap();
        assert_eq!(saved.get("logs"), None);
        assert_eq!(saved.get("incidents"), Some(now));
    }

    #[tokio::test]
    async fn test_next_run_overlaps_previous_window() {
        let path = temp_checkpoint("overlap");
        let fetcher = FakeFetcher::new();
        let sink = FakeSink::default();
        let mut checkpoints = Checkpoints::default();

        let first = ts("2025-01-01T12:00:00Z");
        index_sources(&fetcher, &sink, &mut checkpoints, &path, &config(), first).await;
        let second = ts("2025-01-01T12:15:00Z");
        let mut reloaded = Checkpoints::load(&path, &Source::names()).await.unwrap();
        index_sources(&fetcher, &sink, &mut reloaded, &path, &config(), second).await;

        let window = fetcher.window_of(Source::Logs);
        assert_eq!(window.from, ts("2025-01-01T11:50:00Z"));
        assert_eq!(window.to, second);
    }

    #[tokio::test]
    async fn test_resumes_from_legacy_watermark() {
        let path = temp_checkpoint("legacy");
        tokio::fs::write(&path, "2025-01-01T11:45:00Z")
            .await
            .unwrap();
        let fetcher = FakeFetcher::new();
        let sink = FakeSink::default();
        let mut checkpoints = Checkpoints::load(&path, &Source::names()).await.unwrap();
        let now = ts("2025-01-01T12:00:00Z");

        index_sources(&fetcher, &sink, &mut checkpoints, &path, &config(), now).await;

        for source in [Source::Metrics, Source::Incidents, Source::Logs] {
            assert_eq!(fetcher.window_of(source).from, ts("2025-01-01T11:35:00Z"));
        }
        let saved = Checkpoints::load(&path, &Source::names()).await.unwrap();
        assert_eq!(saved.get("logs"), Some(now));
    }

    #[tokio::test]
    async fn test_overlapping_runs_do_not_duplicate_points() {
        let path = temp_checkpoint("dedupe");
        let mut long_log = doc("log_long", SourceKind::Logs);
        long_log.text = "x".repeat(CHUNK_SIZE * 2);
        let mut fetcher = FakeFetcher::new();
        // The first run sees a duplicate within the page set, the second run re-fetches
        // the overlap plus one new log.
        fetcher.docs.insert(
            "logs",
            vec![
                doc("log_a", SourceKind::Logs),
                long_log.clone(),
                doc("log_a", SourceKind::Logs),
            ],
        );
        let sink = FakeSink::default();
        let mut checkpoints = Checkpoints::default();

        index_sources(
            &fetcher,
            &sink,
            &mut checkpoints,
            &path,
            &config(),
            ts("2025-01-01T12:00:00Z"),
        )
        .await;
        assert_eq!(
            *sink.indexed_ids.lock().unwrap(),
            ["log_a", "log_long"],
            "duplicates within a run are dropped"
        );
        let points_after_first = sink.points.lock().unwrap().len();

        fetcher.docs.insert(
            "logs",
            vec![
                long_log,
                doc("log_a", SourceKind::Logs),
                doc("log_b", SourceKind::Logs),
            ],
        );
        index_sources(
            &fetcher,
            &sink,
            &mut checkpoints,
            &path,
            &config(),
            ts("2025-01-01T12:15:00Z"),
        )
        .await;

        let points = sink.points.lock().unwrap();
        let chunk_ids: HashSet<_> = points.values().cloned().collect();
        assert_eq!(points_after_first, 4); // log_a + 3 chunks of log_long
        assert_eq!(
            points.len(),
            5,
            "re-indexed documents overwrite their points"
        );
        assert_eq!(chunk_ids.len(), points.len());
        assert!(chunk_ids.contains("log_b#c0"));
    }

    #[test]
    fn test_only_configuration_sources_are_fully_synced() {
        for source in Source::ALL {
            let full = matches!(source.sync_scope("run"), SyncScope::Full { .. });
            assert_eq!(
                full,
                matches!(source, Source::Monitors | Source::Dashboards | Source::Slos),
                "{}",
                source.name()
            );
        }
    }

    #[test]
    fn test_index_params_are_bounded() {
        let _guard = ENV_LOCK.lock().unwrap();
        let vars = [
            "INDEXER_EMBED_BATCH_SIZE",
            "INDEXER_EMBED_BATCH_MAX_CHARS",
            "INDEXER_EMBED_CONCURRENCY",
            "INDEXER_ALLOW_EMPTY_SYNC_DELETE",
        ];
        // SAFETY: tests touching the environment hold ENV_LOCK.
        unsafe {
            for v in vars {
                std::env::remove_var(v);
            }
        }
        let defaults = index_params_from_env();
        assert_eq!(defaults.embed_batch_size, EMBED_BATCH_SIZE);
        assert_eq!(defaults.embed_batch_max_chars, EMBED_BATCH_MAX_CHARS);
        assert_eq!(defaults.concurrency, EMBED_CONCURRENCY);
        assert!(!defaults.allow_empty_sync_delete);

        unsafe {
            std::env::set_var("INDEXER_EMBED_BATCH_SIZE", "0");
            std::env::set_var("INDEXER_EMBED_BATCH_MAX_CHARS", "0");
            std::env::set_var("INDEXER_EMBED_CONCURRENCY", "100000");
            std::env::set_var("INDEXER_ALLOW_EMPTY_SYNC_DELETE", "true");
        }
        let clamped = index_params_from_env();
        assert_eq!(clamped.embed_batch_size, 1);
        assert_eq!(clamped.embed_batch_max_chars, 1);
        assert_eq!(clamped.concurrency, MAX_EMBED_CONCURRENCY);
        assert!(clamped.allow_empty_sync_delete);

        unsafe {
            std::env::set_var("INDEXER_EMBED_CONCURRENCY", "0");
        }
        assert_eq!(index_params_from_env().concurrency, 1);
        unsafe {
            for v in vars {
                std::env::remove_var(v);
            }
        }
    }

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn incremental_sink() -> IncrementalSink<FakeEmbedder, FakeStore> {
        IncrementalSink {
            embedder: FakeEmbedder::default(),
            store: FakeStore::default(),
            params: IndexParams {
                chunk_size: CHUNK_SIZE,
                chunk_overlap: CHUNK_OVERLAP,
                embed_batch_size: EMBED_BATCH_SIZE,
                embed_batch_max_chars: EMBED_BATCH_MAX_CHARS,
                concurrency: 2,
                allow_empty_sync_delete: false,
            },
        }
    }

    #[tokio::test]
    async fn test_rerun_embeds_nothing_and_removes_deleted_monitors() {
        let path = temp_checkpoint("incremental");
        let mut fetcher = FakeFetcher::new();
        fetcher.docs.insert(
            "monitors",
            vec![
                doc("monitor_1", SourceKind::Monitor),
                doc("monitor_2", SourceKind::Monitor),
            ],
        );
        fetcher
            .docs
            .insert("logs", vec![doc("log_a", SourceKind::Logs)]);
        let sink = incremental_sink();
        let mut checkpoints = Checkpoints::default();

        let first = ts("2025-01-01T12:00:00Z");
        assert!(
            index_sources(&fetcher, &sink, &mut checkpoints, &path, &config(), first)
                .await
                .is_empty()
        );
        let embedded = sink.embedder.embedded_texts().len();
        assert_eq!(embedded, 3);

        // monitor_2 was deleted in Datadog; the log fell out of the window.
        fetcher
            .docs
            .insert("monitors", vec![doc("monitor_1", SourceKind::Monitor)]);
        fetcher.docs.remove("logs");
        let second = ts("2025-01-01T12:15:00Z");
        assert!(
            index_sources(&fetcher, &sink, &mut checkpoints, &path, &config(), second)
                .await
                .is_empty()
        );

        assert_eq!(sink.embedder.embedded_texts().len(), embedded);
        assert_eq!(sink.store.chunk_ids(), ["log_a#c0", "monitor_1#c0"]);
    }

    /// Serves error logs like Datadog: those in the fetched range, grouped by pattern.
    struct LogFetcher {
        events: Mutex<Vec<log_patterns::LogEvent>>,
    }

    impl SourceFetcher for LogFetcher {
        async fn fetch(&self, source: Source, window: &Window) -> Result<Vec<RagDocument>> {
            if source != Source::Logs {
                return Ok(vec![]);
            }
            let from = log_patterns::fetch_start(window.from);
            let events: Vec<_> = self
                .events
                .lock()
                .unwrap()
                .iter()
                .filter(|e| e.timestamp >= from && e.timestamp <= window.to)
                .cloned()
                .collect();
            Ok(log_patterns::group(&events)
                .iter()
                .map(|p| p.to_document("https://app.datadoghq.eu"))
                .collect())
        }
    }

    fn log_event(id: &str, at: &str) -> log_patterns::LogEvent {
        log_patterns::LogEvent {
            id: id.into(),
            timestamp: ts(at),
            service: "checkout".into(),
            environment: "prod".into(),
            status: "error".into(),
            // Messages differ only in a number, so every log is one pattern.
            message: format!("cache miss for key {}", 4711 * id.len()),
        }
    }

    /// Stored state of the only log pattern point.
    fn stored_pattern(sink: &IncrementalSink<FakeEmbedder, FakeStore>) -> RagDocument {
        let points = sink.store.points.lock().unwrap();
        let logs: Vec<_> = points
            .values()
            .filter(|p| p.payload.kind == SourceKind::Logs)
            .collect();
        assert_eq!(logs.len(), 1, "one point per pattern");
        RagDocument::from(logs[0].payload.clone())
    }

    /// Runs overlap by `INDEXER_OVERLAP_MINUTES`; a pattern seen in both is one point that
    /// counts every log once, including one that reached Datadog after the first run.
    #[tokio::test]
    async fn test_log_patterns_count_each_log_once_across_overlapping_runs() {
        let path = temp_checkpoint("log-patterns");
        let fetcher = LogFetcher {
            events: Mutex::new(vec![
                log_event("1", "2025-01-01T10:45:00Z"),
                log_event("2", "2025-01-01T11:30:00Z"),
                log_event("3", "2025-01-01T11:55:00Z"),
            ]),
        };
        let sink = incremental_sink();
        let mut checkpoints = Checkpoints::default();
        let first = ts("2025-01-01T12:00:00Z");
        assert!(
            index_sources(&fetcher, &sink, &mut checkpoints, &path, &config(), first)
                .await
                .is_empty()
        );
        assert_eq!(stored_pattern(&sink).metadata["count"], 3);

        // Before the next run (window 11:50..12:15:30): a late log inside the overlap and
        // a new one.
        fetcher.events.lock().unwrap().extend([
            log_event("late", "2025-01-01T11:58:00Z"),
            log_event("4", "2025-01-01T12:10:00Z"),
        ]);
        let before_second = checkpoints.clone();
        let second = ts("2025-01-01T12:15:30Z");
        assert!(
            index_sources(&fetcher, &sink, &mut checkpoints, &path, &config(), second)
                .await
                .is_empty()
        );
        let window = checkpoint::window(
            before_second.get("logs"),
            second,
            config().lookback,
            config().overlap,
        );
        assert_eq!(window.from, ts("2025-01-01T11:50:00Z"));
        let doc = stored_pattern(&sink);
        assert_eq!(doc.metadata["count"], 5);
        assert_eq!(doc.metadata["first_seen"], "2025-01-01T10:45:00.000Z");
        assert_eq!(doc.metadata["last_seen"], "2025-01-01T12:10:00.000Z");
        assert_eq!(doc.timestamp.as_deref(), Some("2025-01-01T12:10:00.000Z"));
        assert!(
            doc.text.contains("Occurrences: 5 error log(s)"),
            "{}",
            doc.text
        );

        // The same window again (the checkpoint save failed): nothing changes and nothing
        // is embedded.
        let embedded = sink.embedder.embedded_texts().len();
        let mut retry = before_second;
        index_sources(&fetcher, &sink, &mut retry, &path, &config(), second).await;
        assert_eq!(sink.embedder.embedded_texts().len(), embedded);
        assert_eq!(stored_pattern(&sink).metadata["count"], 5);
    }

    /// Chunks plus the embedding header stay far below the per-request budget and the
    /// per-input token limit (8192 tokens; one char is at most about one token).
    #[test]
    fn test_embedding_inputs_fit_the_batch_budget() {
        let longest = CHUNK_SIZE + rag_core::chunk::EMBEDDING_HEADER_MAX_CHARS;
        assert!(longest < 4000, "{longest}");
        assert!(longest * 16 <= EMBED_BATCH_MAX_CHARS);
        let mut doc = create_test_doc();
        doc.kind = SourceKind::Incident;
        doc.title = "å".repeat(1000);
        doc.service = "s".repeat(500);
        doc.environment = "e".repeat(500);
        doc.metadata
            .insert("severity".into(), "v".repeat(500).into());
        doc.metadata.insert("state".into(), "w".repeat(500).into());
        doc.text = "x".repeat(CHUNK_SIZE * 2);
        for c in chunk(CHUNK_SIZE, CHUNK_OVERLAP, &doc) {
            assert!(rag_core::chunk::embedding_input(&c).chars().count() <= longest);
        }
    }

    #[tokio::test]
    async fn test_failed_fetch_of_full_sync_source_deletes_nothing() {
        let path = temp_checkpoint("full-sync-fetch-failure");
        let mut fetcher = FakeFetcher::new();
        fetcher
            .docs
            .insert("slos", vec![doc("slo_1", SourceKind::SLO)]);
        let sink = incremental_sink();
        let mut checkpoints = Checkpoints::default();
        let first = ts("2025-01-01T12:00:00Z");
        index_sources(&fetcher, &sink, &mut checkpoints, &path, &config(), first).await;

        fetcher.failing = vec![Source::Slos];
        let second = ts("2025-01-01T12:15:00Z");
        let failures =
            index_sources(&fetcher, &sink, &mut checkpoints, &path, &config(), second).await;

        assert_eq!(failures.len(), 1);
        assert_eq!(sink.store.chunk_ids(), ["slo_1#c0"]);
        assert_eq!(checkpoints.get("slos"), Some(first));
    }

    #[tokio::test]
    async fn test_checkpoint_does_not_advance_when_a_delete_fails() {
        let path = temp_checkpoint("delete-failure");
        let mut fetcher = FakeFetcher::new();
        fetcher.docs.insert(
            "dashboards",
            vec![
                doc("dashboard_1", SourceKind::Dashboard),
                doc("dashboard_2", SourceKind::Dashboard),
            ],
        );
        let mut sink = incremental_sink();
        let mut checkpoints = Checkpoints::default();
        let first = ts("2025-01-01T12:00:00Z");
        index_sources(&fetcher, &sink, &mut checkpoints, &path, &config(), first).await;

        fetcher.docs.insert(
            "dashboards",
            vec![doc("dashboard_1", SourceKind::Dashboard)],
        );
        sink.store.fail_deletes = true;
        let second = ts("2025-01-01T12:15:00Z");
        let failures =
            index_sources(&fetcher, &sink, &mut checkpoints, &path, &config(), second).await;

        let failed: Vec<_> = failures.iter().map(|(s, _)| *s).collect();
        assert_eq!(failed, [Source::Dashboards]);
        let saved = Checkpoints::load(&path, &Source::names()).await.unwrap();
        assert_eq!(saved.get("dashboards"), Some(first));
        assert_eq!(saved.get("monitors"), Some(second));
    }
}
