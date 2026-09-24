mod checkpoint;

use anyhow::Result;
use checkpoint::{Checkpoints, Window};
use chrono::{DateTime, Duration, Utc};
use rag_core::{
    chunk::chunk,
    datadog::Datadog,
    domain::RagDocument,
    openai::OpenAiClient,
    qdrant::{QPoint, Qdrant},
};
use std::collections::HashSet;
use std::path::Path;

/// Maximum characters per chunk
const CHUNK_SIZE: usize = 1800;
/// Characters of overlap between consecutive chunks
const CHUNK_OVERLAP: usize = 200;
/// Points per Qdrant upsert request
const UPSERT_BATCH_SIZE: usize = 64;

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
}

/// Fetches the documents of one source for a window.
trait SourceFetcher {
    async fn fetch(&self, source: Source, window: &Window) -> Result<Vec<RagDocument>>;
}

/// Embeds and stores documents, returning the number of chunks written.
trait DocumentSink {
    async fn index(&self, docs: &[RagDocument]) -> Result<usize>;
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
            Source::Logs => self.search_logs(&from_iso, &to_iso).await,
        }
    }
}

struct QdrantSink {
    openai: OpenAiClient,
    qdrant: Qdrant,
}

impl DocumentSink for QdrantSink {
    async fn index(&self, docs: &[RagDocument]) -> Result<usize> {
        let chunks = chunk_documents(docs);
        let mut batch = Vec::new();
        for c in &chunks {
            let emb = self.openai.embed(&c.text).await?;
            batch.push(QPoint::from_document(c, emb));
            if batch.len() >= UPSERT_BATCH_SIZE {
                self.qdrant.upsert(std::mem::take(&mut batch)).await?;
            }
        }
        if !batch.is_empty() {
            self.qdrant.upsert(batch).await?;
        }
        Ok(chunks.len())
    }
}

fn chunk_documents(docs: &[RagDocument]) -> Vec<RagDocument> {
    docs.iter()
        .flat_map(|d| chunk(CHUNK_SIZE, CHUNK_OVERLAP, d))
        .collect()
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
/// its documents are stored. A failing source is logged and skipped so the others still
/// advance; the failures are returned.
async fn index_sources(
    fetcher: &impl SourceFetcher,
    sink: &impl DocumentSink,
    checkpoints: &mut Checkpoints,
    checkpoint_path: &Path,
    config: &IndexerConfig,
    now: DateTime<Utc>,
) -> Vec<(Source, anyhow::Error)> {
    let mut failures = Vec::new();
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

        let result = async {
            let docs = dedupe_by_id(fetcher.fetch(source, &window).await?);
            let chunks = sink.index(&docs).await?;
            Ok::<_, anyhow::Error>((docs.len(), chunks))
        }
        .await;

        match result {
            Ok((docs, chunks)) => {
                tracing::info!(
                    "Indexed {} {} documents ({} chunks)",
                    docs,
                    source.name(),
                    chunks
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
    let sink = QdrantSink {
        openai: OpenAiClient::new_from_env()?,
        qdrant: Qdrant::new_from_env()?,
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
    use rag_core::domain::SourceKind;
    use std::collections::HashMap;
    use std::sync::Mutex;

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
        async fn index(&self, docs: &[RagDocument]) -> Result<usize> {
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
            Ok(chunks.len())
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
}
