//! Incremental indexing: skip unchanged documents, embed changed chunks in bounded
//! concurrent batches, and remove points that no longer belong to a document.
//!
//! Every stored chunk carries its document's content hash and chunk count. Before
//! embedding, the stored state of chunk points `#c0..#c{n}` of each fetched document is
//! read back:
//! - chunks `0..n` all present with the current hash and count: unchanged, not embedded;
//! - otherwise the document is re-embedded and upserted;
//! - a point at `#c{n}` means the document shrank (or an earlier cleanup failed): its
//!   chunks from index `n` on are deleted after the upserts.
//!
//! Stored chunks of a document always form a prefix `0..m`, because chunking numbers
//! them consecutively and deletes remove whole suffixes or whole documents, so probing
//! `#c{n}` is enough to detect surplus chunks.
//!
//! For fully re-synced sources every seen document's points are marked with the run's
//! sync ID, and after all writes succeed, points of that kind without it are deleted.

use anyhow::Result;
use futures::{StreamExt, TryStreamExt, stream};
use rag_core::{
    chunk::{chunk, chunk_id, content_hash, embedding_input},
    domain::{RagDocument, SourceKind},
    openai::{OpenAiClient, embedding_batches},
    qdrant::{
        QPoint, Qdrant, SYNC_ID_KEY, StoredPointState, point_id, surplus_chunks_filter,
        unsynced_filter,
    },
};
use std::collections::HashMap;
use uuid::Uuid;

/// A stored point's `Metadata`.
pub type Metadata = serde_json::Map<String, serde_json::Value>;

/// Points per Qdrant upsert request
const UPSERT_BATCH_SIZE: usize = 64;
/// Point IDs per Qdrant retrieve or set_payload request
const LOOKUP_BATCH_SIZE: usize = 256;

/// Turns chunk texts into vectors.
pub trait Embedder {
    /// Name of the embedding model; part of every content hash.
    fn model(&self) -> &str;
    /// One vector per text, in input order.
    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
}

/// The vector store operations incremental indexing needs.
pub trait PointStore {
    /// Bookkeeping of the points in `ids` that exist.
    async fn retrieve(&self, ids: &[Uuid]) -> Result<Vec<StoredPointState>>;
    /// `Metadata` of the points in `ids` that exist.
    async fn retrieve_metadata(&self, ids: &[Uuid]) -> Result<Vec<(Uuid, Metadata)>>;
    async fn upsert(&self, points: Vec<QPoint>) -> Result<()>;
    /// Deletes chunks `from_index..` of document `doc_id`.
    async fn delete_surplus(
        &self,
        kind: &SourceKind,
        doc_id: &str,
        from_index: usize,
    ) -> Result<()>;
    /// Marks existing points `ids` as seen by run `sync_id`.
    async fn mark_synced(&self, ids: &[Uuid], sync_id: &str) -> Result<()>;
    /// Number of points of `kind` not seen by run `sync_id`.
    async fn count_unsynced(&self, kind: &SourceKind, sync_id: &str) -> Result<u64>;
    /// Deletes the points of `kind` not seen by run `sync_id`.
    async fn delete_unsynced(&self, kind: &SourceKind, sync_id: &str) -> Result<()>;
}

impl Embedder for OpenAiClient {
    fn model(&self) -> &str {
        &self.embedding_model
    }

    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(OpenAiClient::embed_batch(self, texts).await?)
    }
}

impl PointStore for Qdrant {
    async fn retrieve(&self, ids: &[Uuid]) -> Result<Vec<StoredPointState>> {
        Ok(self.retrieve_states(ids).await?)
    }

    async fn retrieve_metadata(&self, ids: &[Uuid]) -> Result<Vec<(Uuid, Metadata)>> {
        Ok(Qdrant::retrieve_metadata(self, ids).await?)
    }

    async fn upsert(&self, points: Vec<QPoint>) -> Result<()> {
        Ok(Qdrant::upsert(self, points).await?)
    }

    async fn delete_surplus(
        &self,
        kind: &SourceKind,
        doc_id: &str,
        from_index: usize,
    ) -> Result<()> {
        Ok(self
            .delete_by_filter(surplus_chunks_filter(kind, doc_id, from_index))
            .await?)
    }

    async fn mark_synced(&self, ids: &[Uuid], sync_id: &str) -> Result<()> {
        Ok(self
            .set_payload(ids, serde_json::json!({ SYNC_ID_KEY: sync_id }))
            .await?)
    }

    async fn count_unsynced(&self, kind: &SourceKind, sync_id: &str) -> Result<u64> {
        Ok(self.count(unsynced_filter(kind, sync_id)).await?)
    }

    async fn delete_unsynced(&self, kind: &SourceKind, sync_id: &str) -> Result<()> {
        Ok(self
            .delete_by_filter(unsynced_filter(kind, sync_id))
            .await?)
    }
}

/// How documents absent from a fetch are treated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncScope {
    /// The fetch covers a time window: absent documents are kept.
    Window,
    /// The fetch returned every document of `kind`: points of `kind` not seen in run
    /// `sync_id` are deleted once all writes succeeded.
    Full { kind: SourceKind, sync_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexParams {
    pub chunk_size: usize,
    pub chunk_overlap: usize,
    /// Most texts per embeddings request.
    pub embed_batch_size: usize,
    /// Most characters per embeddings request (a single longer chunk is sent alone).
    pub embed_batch_max_chars: usize,
    /// Most embedding batches (each followed by its upserts), lookups or deletes in flight.
    pub concurrency: usize,
    /// Delete every point of a full-sync kind when its fetch returns no documents.
    pub allow_empty_sync_delete: bool,
}

/// What indexing one source did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct IndexStats {
    pub fetched: usize,
    pub unchanged: usize,
    pub embedded_docs: usize,
    pub embedded_chunks: usize,
    /// Documents whose surplus chunks were deleted.
    pub shrunk: usize,
    /// Points of documents no longer returned by a full sync.
    pub deleted_stale: u64,
}

/// A fetched document and what has to happen to it.
struct Plan {
    doc_index: usize,
    chunks: Vec<RagDocument>,
    hash: String,
    unchanged: bool,
    has_surplus: bool,
}

/// Embeds and stores documents incrementally; see the module docs.
pub struct IncrementalSink<E, S> {
    pub embedder: E,
    pub store: S,
    pub params: IndexParams,
}

impl<E: Embedder, S: PointStore> IncrementalSink<E, S> {
    fn concurrency(&self) -> usize {
        self.params.concurrency.max(1)
    }

    pub async fn index(&self, docs: &[RagDocument], scope: &SyncScope) -> Result<IndexStats> {
        let plans = self.plan(docs).await?;
        let mut stats = IndexStats {
            fetched: docs.len(),
            ..IndexStats::default()
        };
        let sync_id = match scope {
            SyncScope::Full { sync_id, .. } => Some(sync_id.as_str()),
            SyncScope::Window => None,
        };

        // 1. Embed and upsert the chunks of changed documents.
        let changed: Vec<(&RagDocument, &String, u64)> = plans
            .iter()
            .filter(|p| !p.unchanged)
            .flat_map(|p| {
                let count = p.chunks.len() as u64;
                p.chunks.iter().map(move |c| (c, &p.hash, count))
            })
            .collect();
        stats.unchanged = plans.iter().filter(|p| p.unchanged).count();
        stats.embedded_docs = plans.len() - stats.unchanged;
        stats.embedded_chunks = changed.len();
        // The vector carries a context header (kind, title, service, environment, key
        // metadata); the stored `Text` stays the chunk text.
        let texts: Vec<String> = changed.iter().map(|(c, _, _)| embedding_input(c)).collect();
        let batches = embedding_batches(
            &texts,
            self.params.embed_batch_size,
            self.params.embed_batch_max_chars,
        );
        stream::iter(batches)
            .map(|range| {
                let (chunks, texts) = (&changed[range.clone()], &texts[range]);
                async move {
                    let vectors = self.embedder.embed_batch(texts).await?;
                    anyhow::ensure!(
                        vectors.len() == chunks.len(),
                        "embedder returned {} vectors for {} texts",
                        vectors.len(),
                        chunks.len()
                    );
                    let mut points: Vec<QPoint> = chunks
                        .iter()
                        .zip(vectors)
                        .map(|((c, hash, count), vector)| {
                            let mut point = QPoint::from_document(c, vector);
                            point.payload.content_hash = Some((*hash).clone());
                            point.payload.chunk_count = Some(*count);
                            point.payload.sync_id = sync_id.map(str::to_string);
                            point
                        })
                        .collect();
                    while !points.is_empty() {
                        let rest = points.split_off(points.len().min(UPSERT_BATCH_SIZE));
                        self.store
                            .upsert(std::mem::replace(&mut points, rest))
                            .await?;
                    }
                    Ok::<_, anyhow::Error>(())
                }
            })
            .buffer_unordered(self.concurrency())
            .try_collect::<Vec<()>>()
            .await?;

        // 2. Remove the surplus chunks of documents that shrank.
        let shrunk: Vec<&Plan> = plans.iter().filter(|p| p.has_surplus).collect();
        stats.shrunk = shrunk.len();
        stream::iter(shrunk)
            .map(|p| {
                let doc = &docs[p.doc_index];
                self.store
                    .delete_surplus(&doc.kind, &doc.id, p.chunks.len())
            })
            .buffer_unordered(self.concurrency())
            .try_collect::<Vec<()>>()
            .await?;

        // 3. Full sync: mark unchanged documents as seen, then drop unseen ones.
        if let SyncScope::Full { kind, sync_id } = scope {
            let seen: Vec<Uuid> = plans
                .iter()
                .filter(|p| p.unchanged)
                .flat_map(|p| p.chunks.iter().map(|c| point_id(&c.id)))
                .collect();
            stream::iter(seen.chunks(LOOKUP_BATCH_SIZE))
                .map(|ids| self.store.mark_synced(ids, sync_id))
                .buffer_unordered(self.concurrency())
                .try_collect::<Vec<()>>()
                .await?;
            stats.deleted_stale = self.delete_unseen(kind, sync_id, docs.is_empty()).await?;
        }
        Ok(stats)
    }

    /// The stored metadata of the first chunk of each of `doc_ids` that is indexed.
    pub async fn stored_metadata(&self, doc_ids: &[String]) -> Result<HashMap<String, Metadata>> {
        let by_point: HashMap<Uuid, &String> = doc_ids
            .iter()
            .map(|id| (point_id(&chunk_id(id, 0)), id))
            .collect();
        let ids: Vec<Uuid> = by_point.keys().copied().collect();
        Ok(stream::iter(ids.chunks(LOOKUP_BATCH_SIZE))
            .map(|ids| self.store.retrieve_metadata(ids))
            .buffer_unordered(self.concurrency())
            .try_collect::<Vec<_>>()
            .await?
            .into_iter()
            .flatten()
            .filter_map(|(id, md)| Some((by_point.get(&id)?.to_string(), md)))
            .collect())
    }

    /// Chunks and hashes `docs` and compares them with the stored points.
    async fn plan(&self, docs: &[RagDocument]) -> Result<Vec<Plan>> {
        let (size, overlap) = (self.params.chunk_size, self.params.chunk_overlap);
        let mut plans: Vec<Plan> = docs
            .iter()
            .enumerate()
            .map(|(doc_index, doc)| Plan {
                doc_index,
                chunks: chunk(size, overlap, doc),
                hash: content_hash(doc, size, overlap, self.embedder.model()),
                unchanged: false,
                has_surplus: false,
            })
            .collect();

        // Chunks 0..n plus the probe at n, which exists only if older chunks remain.
        let ids: Vec<Uuid> = plans
            .iter()
            .flat_map(|p| {
                let doc_id = &docs[p.doc_index].id;
                (0..=p.chunks.len()).map(move |i| point_id(&chunk_id(doc_id, i)))
            })
            .collect();
        let stored: HashMap<Uuid, StoredPointState> = stream::iter(ids.chunks(LOOKUP_BATCH_SIZE))
            .map(|ids| self.store.retrieve(ids))
            .buffer_unordered(self.concurrency())
            .try_collect::<Vec<_>>()
            .await?
            .into_iter()
            .flatten()
            .map(|s| (s.id, s))
            .collect();

        for p in &mut plans {
            let doc_id = &docs[p.doc_index].id;
            let count = p.chunks.len() as u64;
            p.unchanged = p.chunks.iter().all(|c| {
                stored.get(&point_id(&c.id)).is_some_and(|s| {
                    s.content_hash.as_deref() == Some(p.hash.as_str())
                        && s.chunk_count == Some(count)
                })
            });
            p.has_surplus = stored.contains_key(&point_id(&chunk_id(doc_id, p.chunks.len())));
        }
        Ok(plans)
    }

    /// Deletes points of `kind` not seen in run `sync_id`, returning how many. An empty
    /// fetch deletes nothing unless explicitly allowed, since it more likely means a
    /// Datadog problem than that every document was removed.
    async fn delete_unseen(&self, kind: &SourceKind, sync_id: &str, empty: bool) -> Result<u64> {
        let stale = self.store.count_unsynced(kind, sync_id).await?;
        if stale == 0 {
            return Ok(0);
        }
        if empty && !self.params.allow_empty_sync_delete {
            tracing::warn!(
                "Fetched no {} documents but {} are indexed; keeping them. Set \
                 INDEXER_ALLOW_EMPTY_SYNC_DELETE=true if they were really all deleted.",
                kind.name(),
                stale
            );
            return Ok(0);
        }
        self.store.delete_unsynced(kind, sync_id).await?;
        Ok(stale)
    }
}

#[cfg(test)]
pub mod fakes {
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Counts calls in flight and remembers the highest count.
    #[derive(Default)]
    pub struct InFlight {
        now: AtomicUsize,
        pub max: AtomicUsize,
    }

    impl InFlight {
        /// Holds a slot across a few scheduler turns so concurrent calls overlap.
        pub async fn hold(&self) {
            let now = self.now.fetch_add(1, Ordering::SeqCst) + 1;
            self.max.fetch_max(now, Ordering::SeqCst);
            for _ in 0..3 {
                tokio::task::yield_now().await;
            }
            self.now.fetch_sub(1, Ordering::SeqCst);
        }

        pub fn max(&self) -> usize {
            self.max.load(Ordering::SeqCst)
        }
    }

    /// Returns a one-element vector per text and records every request.
    pub struct FakeEmbedder {
        pub model: String,
        pub requests: Mutex<Vec<Vec<String>>>,
        pub in_flight: InFlight,
    }

    impl Default for FakeEmbedder {
        fn default() -> Self {
            Self {
                model: "fake-model".into(),
                requests: Mutex::default(),
                in_flight: InFlight::default(),
            }
        }
    }

    impl FakeEmbedder {
        pub fn embedded_texts(&self) -> Vec<String> {
            self.requests.lock().unwrap().concat()
        }
    }

    impl Embedder for FakeEmbedder {
        fn model(&self) -> &str {
            &self.model
        }

        async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
            self.requests.lock().unwrap().push(texts.to_vec());
            self.in_flight.hold().await;
            Ok(texts.iter().map(|_| vec![0.0]).collect())
        }
    }

    /// In-memory stand-in for Qdrant with the same filter semantics.
    #[derive(Default)]
    pub struct FakeStore {
        pub points: Mutex<HashMap<Uuid, QPoint>>,
        pub fail_deletes: bool,
        pub upsert_in_flight: InFlight,
        pub deletes: Mutex<Vec<String>>,
    }

    impl FakeStore {
        /// Logical chunk IDs currently stored, sorted.
        pub fn chunk_ids(&self) -> Vec<String> {
            let mut ids: Vec<String> = self
                .points
                .lock()
                .unwrap()
                .values()
                .map(|p| p.payload.id.clone())
                .collect();
            ids.sort();
            ids
        }

        fn delete_where(&self, what: String, pred: impl Fn(&QPoint) -> bool) -> Result<()> {
            if self.fail_deletes {
                anyhow::bail!("delete failed");
            }
            self.deletes.lock().unwrap().push(what);
            self.points.lock().unwrap().retain(|_, p| !pred(p));
            Ok(())
        }
    }

    fn unsynced(p: &QPoint, kind: &SourceKind, sync_id: &str) -> bool {
        &p.payload.kind == kind && p.payload.sync_id.as_deref() != Some(sync_id)
    }

    impl PointStore for FakeStore {
        async fn retrieve(&self, ids: &[Uuid]) -> Result<Vec<StoredPointState>> {
            let points = self.points.lock().unwrap();
            Ok(ids
                .iter()
                .filter_map(|id| points.get(id))
                .map(|p| StoredPointState {
                    id: p.id,
                    content_hash: p.payload.content_hash.clone(),
                    chunk_count: p.payload.chunk_count,
                })
                .collect())
        }

        async fn retrieve_metadata(&self, ids: &[Uuid]) -> Result<Vec<(Uuid, Metadata)>> {
            let points = self.points.lock().unwrap();
            Ok(ids
                .iter()
                .filter_map(|id| points.get(id))
                .map(|p| (p.id, p.payload.metadata.clone()))
                .collect())
        }

        async fn upsert(&self, points: Vec<QPoint>) -> Result<()> {
            self.upsert_in_flight.hold().await;
            let mut stored = self.points.lock().unwrap();
            for p in points {
                stored.insert(p.id, p);
            }
            Ok(())
        }

        async fn delete_surplus(
            &self,
            kind: &SourceKind,
            doc_id: &str,
            from_index: usize,
        ) -> Result<()> {
            self.delete_where(format!("surplus {doc_id} from {from_index}"), |p| {
                let md = &p.payload.metadata;
                &p.payload.kind == kind
                    && md.get("chunk_of").and_then(|v| v.as_str()) == Some(doc_id)
                    && md
                        .get("chunk_index")
                        .and_then(|v| v.as_u64())
                        .is_some_and(|i| i >= from_index as u64)
            })
        }

        async fn mark_synced(&self, ids: &[Uuid], sync_id: &str) -> Result<()> {
            let mut points = self.points.lock().unwrap();
            for id in ids {
                let point = points
                    .get_mut(id)
                    .ok_or_else(|| anyhow::anyhow!("No point with id {id} found"))?;
                point.payload.sync_id = Some(sync_id.to_string());
            }
            Ok(())
        }

        async fn count_unsynced(&self, kind: &SourceKind, sync_id: &str) -> Result<u64> {
            let points = self.points.lock().unwrap();
            Ok(points
                .values()
                .filter(|p| unsynced(p, kind, sync_id))
                .count() as u64)
        }

        async fn delete_unsynced(&self, kind: &SourceKind, sync_id: &str) -> Result<()> {
            self.delete_where(format!("unsynced {} {sync_id}", kind.name()), |p| {
                unsynced(p, kind, sync_id)
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fakes::*;
    use super::*;

    fn params() -> IndexParams {
        IndexParams {
            chunk_size: 10,
            chunk_overlap: 2,
            embed_batch_size: 4,
            embed_batch_max_chars: 1000,
            concurrency: 2,
            allow_empty_sync_delete: false,
        }
    }

    fn sink() -> IncrementalSink<FakeEmbedder, FakeStore> {
        IncrementalSink {
            embedder: FakeEmbedder::default(),
            store: FakeStore::default(),
            params: params(),
        }
    }

    fn doc(id: &str, kind: SourceKind, text: &str) -> RagDocument {
        RagDocument {
            id: id.into(),
            title: format!("Title {id}"),
            text: text.into(),
            source_uri: format!("https://example.com/{id}"),
            kind,
            timestamp: None,
            service: "svc".into(),
            environment: "prod".into(),
            metadata: serde_json::Map::new(),
        }
    }

    fn full(kind: SourceKind, run: &str) -> SyncScope {
        SyncScope::Full {
            kind,
            sync_id: run.into(),
        }
    }

    #[tokio::test]
    async fn unchanged_documents_are_not_embedded_again() {
        let sink = sink();
        let docs = [
            doc("log_a", SourceKind::Logs, "short"),
            doc("log_b", SourceKind::Logs, "a text of several chunks"),
        ];
        let first = sink.index(&docs, &SyncScope::Window).await.unwrap();
        assert_eq!(first.embedded_docs, 2);
        assert_eq!(first.unchanged, 0);
        let calls = sink.embedder.requests.lock().unwrap().len();

        let second = sink.index(&docs, &SyncScope::Window).await.unwrap();
        assert_eq!(
            second,
            IndexStats {
                fetched: 2,
                unchanged: 2,
                ..IndexStats::default()
            }
        );
        assert_eq!(sink.embedder.requests.lock().unwrap().len(), calls);
        let stored = sink.store.points.lock().unwrap();
        for d in &docs {
            let hash = content_hash(d, 10, 2, "fake-model");
            let chunks = chunk(10, 2, d);
            assert!(chunks.len() > usize::from(d.id == "log_b"));
            for c in &chunks {
                let p = &stored[&point_id(&c.id)].payload;
                assert_eq!(p.content_hash.as_deref(), Some(hash.as_str()));
                assert_eq!(p.chunk_count, Some(chunks.len() as u64));
                assert_eq!(p.sync_id, None);
            }
        }
    }

    #[tokio::test]
    async fn changed_content_or_settings_are_re_embedded() {
        let base = doc("monitor_1", SourceKind::Monitor, "original text");
        let mut text = base.clone();
        text.text = "changed text".into();
        let mut metadata = base.clone();
        metadata.metadata.insert("severity".into(), "SEV-1".into());
        let mut title = base.clone();
        title.title = "Renamed".into();

        for (what, changed, model, chunk_size) in [
            ("text", text, "fake-model", 10),
            ("metadata", metadata, "fake-model", 10),
            ("title", title, "fake-model", 10),
            ("model", base.clone(), "new-model", 10),
            ("chunk size", base.clone(), "fake-model", 12),
        ] {
            let mut sink = sink();
            sink.index(std::slice::from_ref(&base), &SyncScope::Window)
                .await
                .unwrap();
            sink.embedder.requests.lock().unwrap().clear();
            sink.embedder.model = model.into();
            sink.params.chunk_size = chunk_size;

            let stats = sink.index(&[changed], &SyncScope::Window).await.unwrap();
            assert_eq!(stats.embedded_docs, 1, "{what}");
            assert_eq!(stats.unchanged, 0, "{what}");
            assert!(!sink.embedder.embedded_texts().is_empty(), "{what}");
        }
    }

    #[tokio::test]
    async fn points_without_a_hash_count_as_changed() {
        let sink = sink();
        let legacy = doc("log_a", SourceKind::Logs, "short");
        let point = QPoint::from_document(&chunk(10, 2, &legacy)[0], vec![0.0]);
        sink.store.points.lock().unwrap().insert(point.id, point);

        let stats = sink.index(&[legacy], &SyncScope::Window).await.unwrap();
        assert_eq!(stats.embedded_docs, 1);
        assert_eq!(
            sink.embedder.embedded_texts(),
            ["[Log] Title log_a\nservice: svc · env: prod\n\nshort"]
        );
    }

    /// Every chunk is embedded with its document's header; the stored text stays the
    /// chunk text.
    #[tokio::test]
    async fn chunks_are_embedded_with_the_context_header() {
        let sink = sink();
        let mut d = doc(
            "incident_1",
            SourceKind::Incident,
            "first part, second part",
        );
        d.metadata.insert("severity".into(), "SEV-1".into());
        sink.index(std::slice::from_ref(&d), &SyncScope::Window)
            .await
            .unwrap();
        let chunks = chunk(10, 2, &d);
        assert!(chunks.len() > 2);
        let embedded = sink.embedder.embedded_texts();
        assert_eq!(embedded.len(), chunks.len());
        let stored = sink.store.points.lock().unwrap();
        for (c, text) in chunks.iter().zip(&embedded) {
            assert_eq!(
                *text,
                format!(
                    "[Incident] Title incident_1\nservice: svc · env: prod · severity: SEV-1\n\n{}",
                    c.text
                )
            );
            assert_eq!(stored[&point_id(&c.id)].payload.text, c.text);
        }
    }

    #[tokio::test]
    async fn stored_metadata_reads_the_first_chunk_of_indexed_documents() {
        let sink = sink();
        let mut d = doc("logpattern_a", SourceKind::Logs, &"x".repeat(30));
        d.metadata.insert("count".into(), 7.into());
        sink.index(std::slice::from_ref(&d), &SyncScope::Window)
            .await
            .unwrap();
        let found = sink
            .stored_metadata(&["logpattern_a".into(), "logpattern_missing".into()])
            .await
            .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found["logpattern_a"]["count"], 7);
        assert_eq!(found["logpattern_a"]["chunk_index"], 0);
    }

    #[tokio::test]
    async fn a_shrinking_document_deletes_exactly_its_surplus_chunks() {
        let sink = sink();
        let long = doc("log_a", SourceKind::Logs, &"x".repeat(40)); // 5 chunks
        let other = doc("log_b", SourceKind::Logs, &"y".repeat(40));
        sink.index(&[long.clone(), other], &SyncScope::Window)
            .await
            .unwrap();
        assert_eq!(sink.store.chunk_ids().len(), 10);

        let mut short = long;
        short.text = "z".repeat(15); // 2 chunks
        let stats = sink.index(&[short], &SyncScope::Window).await.unwrap();

        assert_eq!(stats.shrunk, 1);
        assert_eq!(
            *sink.store.deletes.lock().unwrap(),
            ["surplus log_a from 2"]
        );
        let ids = sink.store.chunk_ids();
        let a: Vec<_> = ids.iter().filter(|id| id.starts_with("log_a")).collect();
        assert_eq!(a, ["log_a#c0", "log_a#c1"]);
        assert_eq!(ids.iter().filter(|id| id.starts_with("log_b")).count(), 5);

        // Once cleaned up, nothing is left to delete.
        let stats = sink
            .index(
                &[doc("log_a", SourceKind::Logs, &"z".repeat(15))],
                &SyncScope::Window,
            )
            .await
            .unwrap();
        assert_eq!((stats.shrunk, stats.unchanged), (0, 1));
    }

    #[tokio::test]
    async fn a_failed_surplus_cleanup_is_retried_without_re_embedding() {
        let mut sink = sink();
        let long = doc("log_a", SourceKind::Logs, &"x".repeat(40));
        sink.index(std::slice::from_ref(&long), &SyncScope::Window)
            .await
            .unwrap();
        let mut short = long;
        short.text = "z".repeat(15);
        sink.store.fail_deletes = true;
        assert!(
            sink.index(std::slice::from_ref(&short), &SyncScope::Window)
                .await
                .is_err()
        );

        sink.store.fail_deletes = false;
        sink.embedder.requests.lock().unwrap().clear();
        let stats = sink.index(&[short], &SyncScope::Window).await.unwrap();
        assert_eq!((stats.unchanged, stats.shrunk), (1, 1));
        assert!(sink.embedder.embedded_texts().is_empty());
        assert_eq!(sink.store.chunk_ids(), ["log_a#c0", "log_a#c1"]);
    }

    #[tokio::test]
    async fn disappeared_full_sync_documents_are_deleted() {
        for kind in [SourceKind::Monitor, SourceKind::Dashboard, SourceKind::SLO] {
            let sink = sink();
            let keep = doc("keep", kind.clone(), &"k".repeat(30));
            let gone = doc("gone", kind.clone(), "gone");
            let log = doc("log_a", SourceKind::Logs, "a log");
            sink.index(&[log], &SyncScope::Window).await.unwrap();
            sink.index(&[keep.clone(), gone], &full(kind.clone(), "run-1"))
                .await
                .unwrap();

            let stats = sink
                .index(&[keep], &full(kind.clone(), "run-2"))
                .await
                .unwrap();

            assert_eq!(stats.unchanged, 1);
            assert_eq!(stats.deleted_stale, 1);
            assert_eq!(
                sink.store.chunk_ids(),
                ["keep#c0", "keep#c1", "keep#c2", "keep#c3", "log_a#c0"],
                "{kind:?}"
            );
            // Unchanged points were marked as seen, not re-embedded.
            let points = sink.store.points.lock().unwrap();
            assert!(
                points.values().filter(|p| p.payload.kind == kind).all(|p| p
                    .payload
                    .sync_id
                    .as_deref()
                    == Some("run-2"))
            );
        }
    }

    #[tokio::test]
    async fn an_empty_full_sync_deletes_nothing_unless_allowed() {
        let mut sink = sink();
        sink.index(
            &[doc("monitor_1", SourceKind::Monitor, "m")],
            &full(SourceKind::Monitor, "run-1"),
        )
        .await
        .unwrap();

        let stats = sink
            .index(&[], &full(SourceKind::Monitor, "run-2"))
            .await
            .unwrap();
        assert_eq!(stats.deleted_stale, 0);
        assert_eq!(sink.store.chunk_ids(), ["monitor_1#c0"]);
        assert!(sink.store.deletes.lock().unwrap().is_empty());

        sink.params.allow_empty_sync_delete = true;
        let stats = sink
            .index(&[], &full(SourceKind::Monitor, "run-3"))
            .await
            .unwrap();
        assert_eq!(stats.deleted_stale, 1);
        assert!(sink.store.chunk_ids().is_empty());
    }

    #[tokio::test]
    async fn windowed_sources_never_delete_absent_documents() {
        let sink = sink();
        sink.index(
            &[
                doc("log_a", SourceKind::Logs, "a"),
                doc("log_b", SourceKind::Logs, "b"),
            ],
            &SyncScope::Window,
        )
        .await
        .unwrap();
        sink.index(&[doc("log_c", SourceKind::Logs, "c")], &SyncScope::Window)
            .await
            .unwrap();
        sink.index(&[], &SyncScope::Window).await.unwrap();

        assert_eq!(sink.store.chunk_ids(), ["log_a#c0", "log_b#c0", "log_c#c0"]);
        assert!(sink.store.deletes.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn embedding_requests_respect_size_and_char_budget() {
        let mut sink = sink();
        // Ten 10-char chunks from two documents; each input also carries its header, and
        // the budget fits two inputs.
        let docs = [
            doc("log_a", SourceKind::Logs, &"a".repeat(42)),
            doc("log_b", SourceKind::Logs, &"b".repeat(42)),
        ];
        let input = embedding_input(&chunk(10, 2, &docs[0])[0]).chars().count();
        let budget = 2 * input + 1;
        sink.params.embed_batch_size = 3;
        sink.params.embed_batch_max_chars = budget;
        sink.index(&docs, &SyncScope::Window).await.unwrap();

        let requests = sink.embedder.requests.lock().unwrap();
        let total: usize = requests.iter().map(Vec::len).sum();
        assert_eq!(total, sink.store.chunk_ids().len());
        for r in requests.iter() {
            assert!(r.len() <= 3, "{} inputs", r.len());
            let chars: usize = r.iter().map(|t| t.chars().count()).sum();
            assert!(chars <= budget, "{chars} chars");
            assert!(r.len() <= 2, "{} inputs", r.len());
        }
    }

    #[tokio::test]
    async fn concurrency_never_exceeds_the_limit() {
        for limit in [1, 3] {
            let mut sink = sink();
            sink.params.embed_batch_size = 1;
            sink.params.concurrency = limit;
            let docs: Vec<_> = (0..12)
                .map(|i| doc(&format!("log_{i}"), SourceKind::Logs, &format!("text {i}")))
                .collect();
            sink.index(&docs, &SyncScope::Window).await.unwrap();

            assert_eq!(sink.embedder.requests.lock().unwrap().len(), 12);
            assert_eq!(
                sink.embedder.in_flight.max(),
                limit,
                "embeds, limit {limit}"
            );
            assert!(
                sink.store.upsert_in_flight.max() <= limit,
                "upserts, limit {limit}"
            );
        }
    }

    #[tokio::test]
    async fn zero_concurrency_is_treated_as_one() {
        let mut sink = sink();
        sink.params.concurrency = 0;
        sink.params.embed_batch_size = 1;
        let docs = [
            doc("a", SourceKind::Logs, "a"),
            doc("b", SourceKind::Logs, "b"),
        ];
        sink.index(&docs, &SyncScope::Window).await.unwrap();
        assert_eq!(sink.embedder.in_flight.max(), 1);
    }
}
