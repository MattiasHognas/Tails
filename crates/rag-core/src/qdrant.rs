use crate::chunk::embedding_input;
use crate::domain::{Hit, RagDocument, SourceKind};
use crate::error::{RagError, Stage, UpstreamError};
use crate::resilience::{HttpConfig, RetryPolicy, send_with_retry};
use crate::sparse::{SparseVector, document_vector, query_vector, query_vector_without_stopwords};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

/// Name of the dense (embedding) vector of every point.
pub const DENSE_VECTOR: &str = "dense";
/// Name of the sparse keyword vector of every point (see [`crate::sparse`]).
pub const SPARSE_VECTOR: &str = "sparse";
/// Default `k` of reciprocal rank fusion: a point at 0-based rank `r` of a prefetch list
/// scores `1 / (k + r)`. 2 is Qdrant's default, sent explicitly so the fused score (and
/// its normalization in [`Qdrant::hybrid_search`]) cannot change with a Qdrant upgrade.
pub const RRF_K: u32 = 2;

/// Which search a prefetch list of a hybrid query holds, in the request's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListKind {
    Dense,
    Keyword,
}

/// Relative weights of the dense and the keyword lists in reciprocal rank fusion
/// (`RAG_RRF_WEIGHTS=dense,keyword`), both finite and above 0.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RrfWeights {
    pub dense: f32,
    pub keyword: f32,
}

impl RrfWeights {
    fn of(self, list: ListKind) -> f32 {
        match list {
            ListKind::Dense => self.dense,
            ListKind::Keyword => self.keyword,
        }
    }
}

/// Parameters of reciprocal rank fusion (`RAG_RRF_K`, `RAG_RRF_WEIGHTS`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rrf {
    /// At least 1 (Qdrant's minimum).
    pub k: u32,
    /// `None`: every list weighs 1 and the request carries no `weights`.
    pub weights: Option<RrfWeights>,
}

impl Default for Rrf {
    fn default() -> Self {
        Self {
            k: RRF_K,
            weights: None,
        }
    }
}

impl Rrf {
    /// Qdrant's score of 0-based rank `rank` in a list of weight `weight`
    /// (`lib/segment/src/common/reciprocal_rank_fusion.rs`, v1.19):
    /// `1 / ((rank + 1) / weight + k − 1)`, which is `1 / (k + rank)` at weight 1. A
    /// weight divides the 1-based rank: at weight 2 the second point of a list scores
    /// what the first scores at weight 1. So the larger `k`, the less a weight changes
    /// the top of the lists (at k = 60 the first point scores 1/59.5 at weight 2
    /// against 1/60 at weight 1).
    pub fn rank_score(self, rank: usize, weight: f32) -> f32 {
        1.0 / ((rank + 1) as f32 / weight + self.k as f32 - 1.0)
    }

    fn weight(self, list: ListKind) -> f32 {
        self.weights.map_or(1.0, |w| w.of(list))
    }
}

/// How [`Qdrant::hybrid_search`] fuses its dense and keyword prefetch lists
/// (`RAG_FUSION`). See docs/ARCHITECTURE.md#retrieval.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Fusion {
    /// Reciprocal rank fusion (`{"rrf": {"k": k}}`, with `"weights"` per prefetch list
    /// when set): a point at 0-based rank `r` of a list scores
    /// [`Rrf::rank_score`], `1 / (k + r)` unweighted, summed over the lists. Only ranks
    /// count.
    Rrf(Rrf),
    /// Distribution-based score fusion (`{"fusion": "dbsf"}`): Qdrant maps each list's
    /// raw scores to `(s − (μ − 3σ)) / 6σ` (μ and σ the list's mean and sample standard
    /// deviation; not clipped, so a far outlier exceeds 1; 0.5 for a list of one point
    /// or of equal scores) and sums them over the lists. How far apart the raw scores
    /// are counts, not only their order. It takes no parameters.
    Dbsf,
}

impl Default for Fusion {
    fn default() -> Self {
        Fusion::Rrf(Rrf::default())
    }
}

impl Fusion {
    /// `rrf` (with the default parameters) or `dbsf`, case-insensitive.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "rrf" => Some(Fusion::default()),
            "dbsf" => Some(Fusion::Dbsf),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Fusion::Rrf(_) => "rrf",
            Fusion::Dbsf => "dbsf",
        }
    }

    /// The name with any non-default parameters: `rrf`, `rrf(k=10)`,
    /// `rrf(k=2,w=1:2)` (dense:keyword weights), `dbsf`.
    pub fn label(self) -> String {
        match self {
            Fusion::Rrf(Rrf { k, weights: None }) if k == RRF_K => "rrf".into(),
            Fusion::Rrf(Rrf { k, weights: None }) => format!("rrf(k={k})"),
            Fusion::Rrf(Rrf {
                k,
                weights: Some(w),
            }) => format!("rrf(k={k},w={}:{})", w.dense, w.keyword),
            Fusion::Dbsf => "dbsf".into(),
        }
    }

    /// The `query` of the fused request over `lists`.
    fn query(self, lists: &[ListKind]) -> serde_json::Value {
        match self {
            Fusion::Rrf(Rrf { k, weights: None }) => json!({"rrf": {"k": k}}),
            Fusion::Rrf(rrf) => {
                let weights: Vec<f32> = lists.iter().map(|l| rrf.weight(*l)).collect();
                json!({"rrf": {"k": rrf.k, "weights": weights}})
            }
            Fusion::Dbsf => json!({"fusion": "dbsf"}),
        }
    }

    /// The score of a point whose fused score is `raw`, in [0, 1]: `raw` divided by the
    /// best score this fusion gives over the prefetch `lists`, capped at 1.
    ///
    /// - RRF: the best is first in every list, `Σ rank_score(0, weight)`, which is
    ///   `lists / k` unweighted.
    /// - DBSF: the best is 3σ above the mean in every list, `lists`, so the score is
    ///   the mean of the point's normalized scores over the lists (0 for a list it is
    ///   not in, 0.5 at a list's mean). An outlier further out than 3σ exceeds that;
    ///   when the response's `top` point does, every score is divided by `top` instead,
    ///   so the order and ratios of the points stay (the top point scores 1) rather than
    ///   several points being capped at 1 alike. Negative scores (far below a list's
    ///   mean) count as 0.
    pub fn normalize(self, raw: f32, lists: &[ListKind], top: f32) -> f32 {
        let best = match self {
            Fusion::Rrf(rrf) => lists
                .iter()
                .map(|l| rrf.rank_score(0, rrf.weight(*l)))
                .sum(),
            Fusion::Dbsf => (lists.len() as f32).max(top),
        };
        (raw / best).clamp(0.0, 1.0)
    }
}

/// Query-side settings of [`Qdrant::hybrid_search`], from `RAG_FUSION`, `RAG_RRF_K`,
/// `RAG_RRF_WEIGHTS` and `RAG_KEYWORD_STOPWORDS`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct HybridConfig {
    pub fusion: Fusion,
    /// Drop English function words ([`crate::sparse::QUERY_STOPWORDS`]) from the
    /// keyword query vectors. Stored document vectors are never filtered.
    pub query_stopwords: bool,
}

impl HybridConfig {
    /// `RAG_FUSION` (`rrf` | `dbsf`), with RRF's `RAG_RRF_K` (an integer of at least 1,
    /// default 2) and `RAG_RRF_WEIGHTS` (`dense,keyword`, both above 0; default equal,
    /// sent as no weights), and `RAG_KEYWORD_STOPWORDS` (`on` | `off`); unset or empty
    /// means the default. Any other value is an error, as are RRF parameters with
    /// `dbsf`, so a typo cannot silently serve (or measure) another configuration.
    pub fn from_env() -> Result<Self> {
        let d = Self::default();
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        let mut fusion = match var("RAG_FUSION") {
            None => d.fusion,
            Some(v) => Fusion::parse(&v)
                .ok_or_else(|| anyhow::anyhow!("RAG_FUSION must be rrf or dbsf, not {v:?}"))?,
        };
        let k = var("RAG_RRF_K")
            .map(|v| {
                v.trim()
                    .parse::<u32>()
                    .ok()
                    .filter(|k| *k >= 1)
                    .ok_or_else(|| {
                        anyhow::anyhow!("RAG_RRF_K must be an integer of at least 1, not {v:?}")
                    })
            })
            .transpose()?;
        let weights = var("RAG_RRF_WEIGHTS")
            .map(|v| {
                let weight = |w: &str| {
                    w.trim()
                        .parse::<f32>()
                        .ok()
                        .filter(|w| w.is_finite() && *w > 0.0)
                };
                match v.split(',').map(weight).collect::<Vec<_>>().as_slice() {
                    [Some(dense), Some(keyword)] => Ok(RrfWeights {
                        dense: *dense,
                        keyword: *keyword,
                    }),
                    _ => Err(anyhow::anyhow!(
                        "RAG_RRF_WEIGHTS must be two weights above 0, dense,keyword (e.g. 1,2), not {v:?}"
                    )),
                }
            })
            .transpose()?;
        match &mut fusion {
            Fusion::Rrf(rrf) => {
                rrf.k = k.unwrap_or(rrf.k);
                rrf.weights = weights.or(rrf.weights);
            }
            Fusion::Dbsf if k.is_some() || weights.is_some() => {
                anyhow::bail!(
                    "RAG_RRF_K and RAG_RRF_WEIGHTS apply to RAG_FUSION=rrf only, not dbsf"
                )
            }
            Fusion::Dbsf => {}
        }
        let query_stopwords = match var("RAG_KEYWORD_STOPWORDS") {
            None => d.query_stopwords,
            Some(v) => match v.trim().to_ascii_lowercase().as_str() {
                "on" | "true" | "1" | "yes" => true,
                "off" | "false" | "0" | "no" => false,
                _ => anyhow::bail!("RAG_KEYWORD_STOPWORDS must be on or off, not {v:?}"),
            },
        };
        Ok(Self {
            fusion,
            query_stopwords,
        })
    }

    /// The keyword query vector of `text` under this configuration.
    pub fn keyword_query(&self, text: &str) -> SparseVector {
        if self.query_stopwords {
            query_vector_without_stopwords(text)
        } else {
            query_vector(text)
        }
    }

    /// `fusion=rrf stopwords=off` (the fusion's [`Fusion::label`]), for logs and
    /// reports.
    pub fn describe(&self) -> String {
        format!(
            "fusion={} stopwords={}",
            self.fusion.label(),
            if self.query_stopwords { "on" } else { "off" }
        )
    }
}

#[derive(Debug, Clone)]
pub struct Qdrant {
    pub endpoint: String,
    pub collection: String,
    pub http: reqwest::Client,
    pub retry: RetryPolicy,
    /// Fusion and keyword query settings of [`Qdrant::hybrid_search`].
    pub hybrid: HybridConfig,
}

#[derive(Debug, Clone, Serialize)]
pub struct QPoint {
    pub id: Uuid,
    pub vector: PointVectors,
    pub payload: QdrantPayload,
}

/// The named vectors of a point. The field names are [`DENSE_VECTOR`] and
/// [`SPARSE_VECTOR`], the names [`collection_config`] creates and
/// [`Qdrant::hybrid_search`] queries.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PointVectors {
    pub dense: Vec<f32>,
    pub sparse: SparseVector,
}

/// Body of `PUT /collections/{c}`: a cosine dense vector of `dim` dimensions and a
/// sparse vector whose IDF Qdrant computes over the collection.
pub fn collection_config(dim: usize) -> serde_json::Value {
    json!({
        "vectors": {DENSE_VECTOR: {"size": dim, "distance": "Cosine"}},
        "sparse_vectors": {SPARSE_VECTOR: {"modifier": "idf"}}
    })
}

/// Checks the `config.params` of an existing collection against [`collection_config`]
/// (any dimension). The error says what differs.
pub fn check_collection_params(params: &serde_json::Value) -> Result<(), String> {
    let dense = &params["vectors"][DENSE_VECTOR];
    if !dense.is_object() {
        return Err(format!(
            "no named dense vector `{DENSE_VECTOR}` (vectors: {})",
            params["vectors"]
        ));
    }
    if dense["distance"] != "Cosine" {
        return Err(format!(
            "dense vector `{DENSE_VECTOR}` uses distance {}, not Cosine",
            dense["distance"]
        ));
    }
    let sparse = &params["sparse_vectors"][SPARSE_VECTOR];
    if !sparse.is_object() {
        return Err(format!("no sparse vector `{SPARSE_VECTOR}`"));
    }
    if sparse["modifier"] != "idf" {
        return Err(format!(
            "sparse vector `{SPARSE_VECTOR}` has modifier {}, not idf",
            sparse["modifier"]
        ));
    }
    Ok(())
}

/// One way of asking a question: its embedding and its keyword vector
/// ([`crate::sparse::query_vector`]).
#[derive(Debug, Clone, PartialEq)]
pub struct SearchQuery {
    pub dense: Vec<f32>,
    pub sparse: SparseVector,
}

/// Body of the hybrid `POST /collections/{c}/points/query` and the prefetch lists it
/// fuses, in order: per query a dense and (unless it has no tokens) a sparse prefetch,
/// each restricted by `filter` and returning up to `limit` points, fused with `fusion`.
pub fn hybrid_query_body(
    queries: &[SearchQuery],
    limit: usize,
    filter: Option<&serde_json::Value>,
    fusion: Fusion,
) -> (serde_json::Value, Vec<ListKind>) {
    let prefetch_of = |query: serde_json::Value, using: &str| {
        let mut p = json!({"query": query, "using": using, "limit": limit});
        if let Some(f) = filter {
            p["filter"] = f.clone();
        }
        p
    };
    let mut prefetch = vec![];
    let mut lists = vec![];
    for q in queries {
        prefetch.push(prefetch_of(json!(q.dense), DENSE_VECTOR));
        lists.push(ListKind::Dense);
        if !q.sparse.is_empty() {
            prefetch.push(prefetch_of(json!(q.sparse), SPARSE_VECTOR));
            lists.push(ListKind::Keyword);
        }
    }
    let body = json!({
        "prefetch": prefetch,
        "query": fusion.query(&lists),
        "limit": limit,
        "with_payload": true,
    });
    (body, lists)
}

/// Storage schema shared by ingestion and retrieval. PascalCase keys match
/// Qdrant filters; the public RagDocument uses camelCase.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct QdrantPayload {
    #[serde(rename = "id")]
    pub id: String,
    pub title: String,
    pub text: String,
    pub source_uri: String,
    pub kind: SourceKind,
    pub timestamp: Option<String>,
    pub service: String,
    pub environment: String,
    #[serde(default)]
    pub metadata: serde_json::Map<String, serde_json::Value>,
    // Indexer bookkeeping. Points written before incremental indexing lack these keys
    // and count as changed. Not part of `RagDocument`, so never shown to the LLM.
    /// [`crate::chunk::content_hash`] of the parent document.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    /// Number of chunks of the parent document.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chunk_count: Option<u64>,
    /// Fully re-synced kinds only: ID of the last indexer run that saw the document.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync_id: Option<String>,
}

/// Payload keys of the indexer bookkeeping fields.
pub const CONTENT_HASH_KEY: &str = "ContentHash";
pub const CHUNK_COUNT_KEY: &str = "ChunkCount";
pub const SYNC_ID_KEY: &str = "SyncId";

impl From<&RagDocument> for QdrantPayload {
    fn from(doc: &RagDocument) -> Self {
        Self {
            id: doc.id.clone(),
            title: doc.title.clone(),
            text: doc.text.clone(),
            source_uri: doc.source_uri.clone(),
            kind: doc.kind.clone(),
            timestamp: doc.timestamp.clone(),
            service: doc.service.clone(),
            environment: doc.environment.clone(),
            metadata: doc.metadata.clone(),
            content_hash: None,
            chunk_count: None,
            sync_id: None,
        }
    }
}

impl From<QdrantPayload> for RagDocument {
    fn from(payload: QdrantPayload) -> Self {
        Self {
            id: payload.id,
            title: payload.title,
            text: payload.text,
            source_uri: payload.source_uri,
            kind: payload.kind,
            timestamp: payload.timestamp,
            service: payload.service,
            environment: payload.environment,
            metadata: payload.metadata,
        }
    }
}

impl QPoint {
    /// A chunk keeps the same point ID across retries and content updates.
    /// Preserve the logical chunk ID and chunk_of metadata in the payload.
    ///
    /// `dense` is the embedding of [`embedding_input`]; the sparse vector is built from
    /// the same text, so both searches see the header and the chunk.
    pub fn from_document(doc: &RagDocument, dense: Vec<f32>) -> Self {
        Self {
            id: point_id(&doc.id),
            vector: PointVectors {
                dense,
                sparse: document_vector(&embedding_input(doc)),
            },
            payload: payload_from(doc),
        }
    }
}

/// Deterministic point ID of a logical chunk ID (for example `monitor_123#c0`).
pub fn point_id(logical_id: &str) -> Uuid {
    // This namespace is part of the persisted ID contract: do not change it.
    let namespace = Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        b"https://github.com/MattiasHognas/Tails",
    );
    Uuid::new_v5(&namespace, logical_id.as_bytes())
}

pub fn payload_from(doc: &RagDocument) -> QdrantPayload {
    QdrantPayload::from(doc)
}

/// Indexer bookkeeping read back from a stored point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredPointState {
    pub id: Uuid,
    pub content_hash: Option<String>,
    pub chunk_count: Option<u64>,
}

/// Points of document `doc_id` (of `kind`) whose chunk index is `from_index` or higher:
/// the surplus chunks left when a document shrinks.
pub fn surplus_chunks_filter(
    kind: &SourceKind,
    doc_id: &str,
    from_index: usize,
) -> serde_json::Value {
    serde_json::json!({"must": [
        {"key": "Kind", "match": {"value": kind.payload_value()}},
        {"key": "Metadata.chunk_of", "match": {"value": doc_id}},
        {"key": "Metadata.chunk_index", "range": {"gte": from_index}}
    ]})
}

/// Points of `kind` not marked with `sync_id`, including points without a marker.
pub fn unsynced_filter(kind: &SourceKind, sync_id: &str) -> serde_json::Value {
    serde_json::json!({
        "must": [{"key": "Kind", "match": {"value": kind.payload_value()}}],
        "must_not": [{"key": SYNC_ID_KEY, "match": {"value": sync_id}}]
    })
}

impl Qdrant {
    pub fn new(endpoint: String, collection: String) -> Self {
        Self {
            endpoint,
            collection,
            http: HttpConfig::default().build_client(),
            retry: RetryPolicy::default(),
            hybrid: HybridConfig::default(),
        }
    }

    pub fn new_from_env() -> Result<Self> {
        let mut qdrant = Self::new(
            std::env::var("QDRANT_ENDPOINT").unwrap_or_else(|_| "http://localhost:6333".into()),
            std::env::var("QDRANT_COLLECTION").unwrap_or_else(|_| "datadog_rag".into()),
        );
        qdrant.http = HttpConfig::from_env().build_client();
        qdrant.retry = RetryPolicy::from_env();
        qdrant.hybrid = HybridConfig::from_env()?;
        Ok(qdrant)
    }

    pub async fn upsert(&self, points: Vec<QPoint>) -> Result<(), RagError> {
        #[derive(Serialize)]
        struct Req {
            points: Vec<QPoint>,
        }
        let url = format!(
            "{}/collections/{}/points?wait=true",
            self.endpoint, self.collection
        );
        let req = Req { points };
        send_with_retry(&self.retry, "qdrant upsert", || {
            self.http.put(&url).json(&req)
        })
        .await
        .map_err(|f| RagError::upstream(Stage::Indexing, f))?;
        Ok(())
    }

    fn points_url(&self, suffix: &str) -> String {
        format!(
            "{}/collections/{}/points{}",
            self.endpoint, self.collection, suffix
        )
    }

    /// POST `body` to the points endpoint `suffix` for the indexer and decode the response.
    async fn post_points<B: Serialize, T: serde::de::DeserializeOwned>(
        &self,
        what: &str,
        suffix: &str,
        body: &B,
    ) -> Result<T, RagError> {
        let url = self.points_url(suffix);
        let r = send_with_retry(&self.retry, what, || self.http.post(&url).json(body))
            .await
            .map_err(|f| RagError::upstream(Stage::Indexing, f))?;
        r.json()
            .await
            .map_err(|e| RagError::failed(Stage::Indexing, UpstreamError::from_reqwest(e)))
    }

    /// Reads the indexer bookkeeping of the points in `ids` that exist; missing IDs are
    /// left out of the result.
    pub async fn retrieve_states(&self, ids: &[Uuid]) -> Result<Vec<StoredPointState>, RagError> {
        #[derive(Deserialize)]
        struct Resp {
            result: Vec<Item>,
        }
        #[derive(Deserialize)]
        struct Item {
            id: Uuid,
            #[serde(default)]
            payload: Option<State>,
        }
        #[derive(Default, Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct State {
            content_hash: Option<String>,
            chunk_count: Option<u64>,
        }
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let req = serde_json::json!({
            "ids": ids,
            "with_payload": [CONTENT_HASH_KEY, CHUNK_COUNT_KEY],
            "with_vector": false,
        });
        let v: Resp = self.post_points("qdrant retrieve", "", &req).await?;
        Ok(v.result
            .into_iter()
            .map(|it| {
                let state = it.payload.unwrap_or_default();
                StoredPointState {
                    id: it.id,
                    content_hash: state.content_hash,
                    chunk_count: state.chunk_count,
                }
            })
            .collect())
    }

    /// Reads the `Metadata` of the points in `ids` that exist; missing IDs are left out
    /// of the result. The indexer merges stored log pattern counts with it.
    pub async fn retrieve_metadata(
        &self,
        ids: &[Uuid],
    ) -> Result<Vec<(Uuid, serde_json::Map<String, serde_json::Value>)>, RagError> {
        #[derive(Deserialize)]
        struct Resp {
            result: Vec<Item>,
        }
        #[derive(Deserialize)]
        struct Item {
            id: Uuid,
            #[serde(default)]
            payload: Option<Payload>,
        }
        #[derive(Default, Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct Payload {
            #[serde(default)]
            metadata: serde_json::Map<String, serde_json::Value>,
        }
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let req = serde_json::json!({
            "ids": ids,
            "with_payload": ["Metadata"],
            "with_vector": false,
        });
        let v: Resp = self.post_points("qdrant retrieve", "", &req).await?;
        Ok(v.result
            .into_iter()
            .map(|it| (it.id, it.payload.unwrap_or_default().metadata))
            .collect())
    }

    /// Merges `payload` into the existing points `ids`. Every ID must exist.
    pub async fn set_payload(
        &self,
        ids: &[Uuid],
        payload: serde_json::Value,
    ) -> Result<(), RagError> {
        if ids.is_empty() {
            return Ok(());
        }
        let req = serde_json::json!({"payload": payload, "points": ids});
        let _: serde_json::Value = self
            .post_points("qdrant set_payload", "/payload?wait=true", &req)
            .await?;
        Ok(())
    }

    /// Deletes every point matching `filter`.
    pub async fn delete_by_filter(&self, filter: serde_json::Value) -> Result<(), RagError> {
        let req = serde_json::json!({"filter": filter});
        let _: serde_json::Value = self
            .post_points("qdrant delete", "/delete?wait=true", &req)
            .await?;
        Ok(())
    }

    /// Exact number of points matching `filter`.
    pub async fn count(&self, filter: serde_json::Value) -> Result<u64, RagError> {
        #[derive(Deserialize)]
        struct Resp {
            result: Count,
        }
        #[derive(Deserialize)]
        struct Count {
            count: u64,
        }
        let req = serde_json::json!({"filter": filter, "exact": true});
        let v: Resp = self.post_points("qdrant count", "/count", &req).await?;
        Ok(v.result.count)
    }

    /// Creates the collection with [`collection_config`] for `dim`-dimensional
    /// embeddings.
    pub async fn create_collection(&self, dim: usize) -> Result<(), RagError> {
        let url = format!("{}/collections/{}", self.endpoint, self.collection);
        let body = collection_config(dim);
        send_with_retry(&self.retry, "qdrant create collection", || {
            self.http.put(&url).json(&body)
        })
        .await
        .map_err(|f| RagError::upstream(Stage::Indexing, f))?;
        Ok(())
    }

    /// Whether the collection exists. An existing collection must have the layout of
    /// [`collection_config`]; one without it (for example the old single unnamed
    /// vector) is an error, because it cannot be migrated in place.
    pub async fn check_collection(&self) -> Result<bool, RagError> {
        #[derive(Deserialize)]
        struct Resp {
            result: Info,
        }
        #[derive(Deserialize)]
        struct Info {
            config: Config,
        }
        #[derive(Deserialize)]
        struct Config {
            params: serde_json::Value,
        }
        let url = format!("{}/collections/{}", self.endpoint, self.collection);
        let r = match send_with_retry(&self.retry, "qdrant collection info", || {
            self.http.get(&url)
        })
        .await
        {
            Ok(r) => r,
            Err(f) if matches!(f.error, UpstreamError::Status { status: 404, .. }) => {
                return Ok(false);
            }
            Err(f) => return Err(RagError::upstream(Stage::Indexing, f)),
        };
        let info: Resp = r
            .json()
            .await
            .map_err(|e| RagError::failed(Stage::Indexing, UpstreamError::from_reqwest(e)))?;
        check_collection_params(&info.result.config.params).map_err(|why| {
            RagError::failed(
                Stage::Indexing,
                UpstreamError::InvalidResponse(format!(
                    "collection {} is not a hybrid search collection: {why}. Index into a new \
                     collection (QDRANT_COLLECTION); older layouts are not migrated",
                    self.collection
                )),
            )
        })?;
        Ok(true)
    }

    /// Hybrid search: dense and keyword search for every query, each restricted by
    /// `filter`, fused by Qdrant with [`HybridConfig::fusion`] (see
    /// [`hybrid_query_body`]).
    ///
    /// Returns at most `limit` hits, with scores in [0, 1] ([`Fusion::normalize`]):
    /// - RRF (default): the fused score divided by the best possible one (first in
    ///   every prefetch list): 1 means ranked first by every search, 0.667 second by
    ///   all, 0.5 first by half of them or third by all (with [`RRF_K`] = 2 and equal
    ///   weights).
    /// - DBSF: the mean over the lists of the point's normalized score: 0.5 at a list's
    ///   mean, 1 at 3σ above it, 0 in a list it is not in.
    ///
    /// Either way the scale does not depend on the units of the raw similarities, so
    /// the reranker's kind priors and recency weight
    /// ([`crate::reranker::recency_weight`]) scale every question's scores alike.
    pub async fn hybrid_search(
        &self,
        queries: &[SearchQuery],
        limit: usize,
        filter: Option<serde_json::Value>,
    ) -> Result<Vec<Hit>, RagError> {
        #[derive(Deserialize)]
        struct Resp {
            result: Points,
        }
        #[derive(Deserialize)]
        struct Points {
            points: Vec<Item>,
        }
        #[derive(Deserialize)]
        struct Item {
            score: f32,
            payload: QdrantPayload,
        }
        let (req, lists) = hybrid_query_body(queries, limit, filter.as_ref(), self.hybrid.fusion);
        if lists.is_empty() {
            return Err(RagError::failed(
                Stage::Retrieval,
                UpstreamError::InvalidResponse("hybrid search without a query".into()),
            ));
        }
        let url = self.points_url("/query");
        let r = send_with_retry(&self.retry, "qdrant query", || {
            self.http.post(&url).json(&req)
        })
        .await
        .map_err(|f| RagError::upstream(Stage::Retrieval, f))?;
        // A malformed payload is a retrieval failure, never "no evidence".
        let v: Resp = r
            .json()
            .await
            .map_err(|e| RagError::failed(Stage::Retrieval, UpstreamError::from_reqwest(e)))?;
        let fusion = self.hybrid.fusion;
        let top = v.result.points.first().map_or(0.0, |p| p.score);
        Ok(v.result
            .points
            .into_iter()
            .map(|it| Hit {
                doc: it.payload.into(),
                score: fusion.normalize(it.score, &lists, top),
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::SourceKind;
    use crate::test_support::lock_env;

    fn contract_document() -> RagDocument {
        RagDocument {
            id: "monitor_123#c0".into(),
            title: "Återkommande fel".into(),
            text: "Connection failed".into(),
            source_uri: "https://app.datadoghq.eu/monitors/123".into(),
            kind: SourceKind::Monitor,
            timestamp: None,
            service: "payments".into(),
            environment: "prod".into(),
            metadata: serde_json::json!({"chunk_of": "monitor_123", "chunk_index": 0})
                .as_object()
                .unwrap()
                .clone(),
        }
    }

    #[test]
    fn point_ids_are_stable_namespaced_uuids() {
        let mut doc = contract_document();
        let first = QPoint::from_document(&doc, vec![1.0]);
        // Pin the persisted mapping so refactors cannot silently duplicate points.
        assert_eq!(first.id.to_string(), "21aca85b-31bc-5c36-bd5f-32403cb008d2");
        doc.text = "Changed content".into();
        assert_eq!(first.id, QPoint::from_document(&doc, vec![0.5]).id);
        doc.id = "monitor_123#c1".into();
        assert_ne!(first.id, QPoint::from_document(&doc, vec![1.0]).id);
        doc.id = "incident_123#c0".into();
        assert_ne!(first.id, QPoint::from_document(&doc, vec![1.0]).id);
        assert_eq!(first.payload.id, "monitor_123#c0");
        assert_eq!(first.payload.metadata["chunk_of"], "monitor_123");
    }

    #[test]
    fn typed_payload_roundtrips_every_source_kind() {
        for kind in [
            SourceKind::Logs,
            SourceKind::Metrics,
            SourceKind::Monitor,
            SourceKind::Incident,
            SourceKind::Dashboard,
            SourceKind::SLO,
            SourceKind::Git,
        ] {
            let mut doc = contract_document();
            doc.kind = kind;
            let value = serde_json::to_value(payload_from(&doc)).unwrap();
            let payload: QdrantPayload = serde_json::from_value(value).unwrap();
            let restored = RagDocument::from(payload);
            assert_eq!(
                serde_json::to_value(restored).unwrap(),
                serde_json::to_value(doc).unwrap()
            );
        }
    }

    #[tokio::test]
    async fn search_rejects_incomplete_payloads_instead_of_returning_empty_evidence() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let mut payload = serde_json::to_value(payload_from(&contract_document())).unwrap();
        payload.as_object_mut().unwrap().remove("Text");
        Mock::given(method("POST"))
            .and(path("/collections/test/points/query"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": {"points": [{"id": 1, "score": 0.9, "payload": payload}]}
            })))
            .mount(&server)
            .await;
        let qdrant = Qdrant::new(server.uri(), "test".into());
        assert!(
            qdrant
                .hybrid_search(&[query(&[1.0], "x")], 10, None)
                .await
                .is_err()
        );
    }

    fn query(dense: &[f32], text: &str) -> SearchQuery {
        SearchQuery {
            dense: dense.to_vec(),
            sparse: crate::sparse::query_vector(text),
        }
    }

    #[test]
    fn collection_config_names_a_dense_and_an_idf_sparse_vector() {
        let config = collection_config(1536);
        assert_eq!(
            config,
            serde_json::json!({
                "vectors": {"dense": {"size": 1536, "distance": "Cosine"}},
                "sparse_vectors": {"sparse": {"modifier": "idf"}}
            })
        );
        // What Qdrant reports back for it passes the check.
        assert_eq!(check_collection_params(&config), Ok(()));
    }

    #[test]
    fn collections_without_the_hybrid_layout_are_rejected() {
        let old = serde_json::json!({"vectors": {"size": 1536, "distance": "Cosine"}});
        assert!(check_collection_params(&old).unwrap_err().contains("dense"));
        let mut dot = collection_config(3);
        dot["vectors"]["dense"]["distance"] = "Dot".into();
        assert!(
            check_collection_params(&dot)
                .unwrap_err()
                .contains("Cosine")
        );
        let mut no_sparse = collection_config(3);
        no_sparse.as_object_mut().unwrap().remove("sparse_vectors");
        assert!(
            check_collection_params(&no_sparse)
                .unwrap_err()
                .contains("sparse")
        );
        let mut no_idf = collection_config(3);
        no_idf["sparse_vectors"]["sparse"] = serde_json::json!({});
        assert!(
            check_collection_params(&no_idf)
                .unwrap_err()
                .contains("idf")
        );
    }

    #[tokio::test]
    async fn check_collection_reports_missing_valid_and_old_collections() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let info = |params: serde_json::Value| {
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": {"status": "green", "config": {"params": params}}, "status": "ok"
            }))
        };
        Mock::given(method("GET"))
            .and(path("/collections/hybrid"))
            .respond_with(info(collection_config(3)))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/collections/old"))
            .respond_with(info(
                serde_json::json!({"vectors": {"size": 3, "distance": "Cosine"}}),
            ))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/collections/missing"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path("/collections/missing"))
            .and(wiremock::matchers::body_json(collection_config(3)))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": true, "status": "ok"
            })))
            .expect(1)
            .mount(&server)
            .await;
        let named = |c: &str| {
            let mut q = mock_qdrant(server.uri());
            q.collection = c.into();
            q
        };
        assert!(named("hybrid").check_collection().await.unwrap());
        assert!(!named("missing").check_collection().await.unwrap());
        named("missing").create_collection(3).await.unwrap();
        let err = named("old").check_collection().await.unwrap_err();
        assert!(matches!(err, RagError::IndexingFailed { .. }), "{err:?}");
        assert!(err.to_string().contains("new"), "{err}");
    }

    #[test]
    fn test_qdrant_new_from_env_defaults() {
        let _env_lock = lock_env();

        unsafe {
            std::env::remove_var("QDRANT_ENDPOINT");
            std::env::remove_var("QDRANT_COLLECTION");
        }

        let qdrant = Qdrant::new_from_env().unwrap();
        assert_eq!(qdrant.endpoint, "http://localhost:6333");
        assert_eq!(qdrant.collection, "datadog_rag");
    }

    #[test]
    fn hybrid_config_reads_fusion_and_stopwords_from_env() {
        use crate::test_support::EnvVarGuard;
        let _env_lock = lock_env();
        let _guards = [
            "RAG_FUSION",
            "RAG_KEYWORD_STOPWORDS",
            "RAG_RRF_K",
            "RAG_RRF_WEIGHTS",
        ]
        .map(EnvVarGuard::preserve);
        let set = |name: &str, value: Option<&str>| unsafe {
            match value {
                Some(v) => std::env::set_var(name, v),
                None => std::env::remove_var(name),
            }
        };
        let set_all = |fusion, stopwords, k, weights| {
            set("RAG_FUSION", fusion);
            set("RAG_KEYWORD_STOPWORDS", stopwords);
            set("RAG_RRF_K", k);
            set("RAG_RRF_WEIGHTS", weights);
        };
        set_all(None, None, None, None);
        assert_eq!(HybridConfig::from_env().unwrap(), HybridConfig::default());
        assert_eq!(
            Qdrant::new_from_env().unwrap().hybrid,
            HybridConfig::default()
        );
        assert_eq!(
            HybridConfig::default().describe(),
            "fusion=rrf stopwords=off"
        );
        set_all(Some(" DBSF "), Some("on"), None, None);
        let c = HybridConfig::from_env().unwrap();
        assert_eq!(
            c,
            HybridConfig {
                fusion: Fusion::Dbsf,
                query_stopwords: true
            }
        );
        assert_eq!(c.describe(), "fusion=dbsf stopwords=on");
        assert_eq!(Qdrant::new_from_env().unwrap().hybrid, c);
        set_all(Some("rrf"), Some("off"), None, None);
        assert_eq!(
            HybridConfig::from_env().unwrap(),
            HybridConfig {
                fusion: Fusion::Rrf(Rrf::default()),
                query_stopwords: false
            }
        );
        // RRF parameters, with or without RAG_FUSION=rrf.
        set_all(None, None, Some(" 60 "), None);
        let c = HybridConfig::from_env().unwrap();
        assert_eq!(
            c.fusion,
            Fusion::Rrf(Rrf {
                k: 60,
                weights: None
            })
        );
        assert_eq!(c.describe(), "fusion=rrf(k=60) stopwords=off");
        set_all(Some("rrf"), None, Some("2"), Some("1, 2.5"));
        let c = HybridConfig::from_env().unwrap();
        assert_eq!(
            c.fusion,
            Fusion::Rrf(Rrf {
                k: 2,
                weights: Some(RrfWeights {
                    dense: 1.0,
                    keyword: 2.5
                })
            })
        );
        assert_eq!(c.describe(), "fusion=rrf(k=2,w=1:2.5) stopwords=off");
        // Empty is unset; a typo is an error, never a silent default.
        set_all(Some(""), Some(" "), Some(""), Some(" "));
        assert_eq!(HybridConfig::from_env().unwrap(), HybridConfig::default());
        let fails = |fusion, stopwords, k, weights, name: &str| {
            set_all(fusion, stopwords, k, weights);
            let err = HybridConfig::from_env().unwrap_err().to_string();
            assert!(err.contains(name), "{err}");
        };
        fails(Some("rfr"), None, None, None, "RAG_FUSION");
        assert!(Qdrant::new_from_env().is_err());
        fails(None, Some("maybe"), None, None, "RAG_KEYWORD_STOPWORDS");
        for k in ["0", "-1", "2.5", "ten"] {
            fails(None, None, Some(k), None, "RAG_RRF_K");
        }
        for w in ["2", "1,2,3", "1,0", "-1,1", "1,x", "1,inf", "1,,2"] {
            fails(None, None, None, Some(w), "RAG_RRF_WEIGHTS");
        }
        // DBSF takes no parameters (Qdrant ignores any), so asking for them is an error.
        fails(Some("dbsf"), None, Some("10"), None, "dbsf");
        fails(Some("dbsf"), None, None, Some("1,2"), "dbsf");
    }

    #[test]
    fn keyword_queries_drop_stopwords_only_when_configured() {
        let q = "Which errors did the inventory service log?";
        let off = HybridConfig::default();
        let on = HybridConfig {
            query_stopwords: true,
            ..off
        };
        assert_eq!(off.keyword_query(q), query_vector(q));
        assert_eq!(
            on.keyword_query(q),
            query_vector("errors inventory service log")
        );
    }

    #[test]
    fn fused_scores_are_normalized_per_fusion() {
        use ListKind::{Dense, Keyword};
        let rrf = Fusion::default();
        // RRF: best is first in every list, lists / k.
        let three = [Dense, Keyword, Dense];
        assert_eq!(rrf.normalize(1.5, &three, 1.5), 1.0);
        assert!((rrf.normalize(1.0 / 3.0, &three, 1.5) - 2.0 / 9.0).abs() < 1e-6);
        let k10 = Fusion::Rrf(Rrf {
            k: 10,
            weights: None,
        });
        assert!((k10.normalize(0.1, &three, 0.1) - 1.0 / 3.0).abs() < 1e-6);
        // Weighted: first in a list of weight w scores 1 / (1/w + k - 1). Keyword 2,
        // k = 2: 1/1.5 per keyword list, 1/2 per dense list.
        let weighted = Fusion::Rrf(Rrf {
            k: 2,
            weights: Some(RrfWeights {
                dense: 1.0,
                keyword: 2.0,
            }),
        });
        let best = 0.5 + 1.0 / 1.5;
        assert!((weighted.normalize(best, &[Dense, Keyword], 0.0) - 1.0).abs() < 1e-6);
        assert!((weighted.normalize(0.5, &[Dense, Keyword], 0.0) - 0.5 / best).abs() < 1e-6);
        // DBSF: the mean of the per-list normalized scores...
        let two = [Dense, Keyword];
        assert!((Fusion::Dbsf.normalize(1.5, &two, 1.5) - 0.75).abs() < 1e-6);
        assert!((Fusion::Dbsf.normalize(0.5, &two, 1.5) - 0.25).abs() < 1e-6);
        // ...unless the top point is beyond 3σ on average: then relative to the top,
        // keeping the ratios instead of capping several points at 1.
        assert_eq!(Fusion::Dbsf.normalize(2.5, &two, 2.5), 1.0);
        assert!((Fusion::Dbsf.normalize(2.25, &two, 2.5) - 0.9).abs() < 1e-6);
        assert!((Fusion::Dbsf.normalize(1.0, &two, 2.5) - 0.4).abs() < 1e-6);
        // Far below every list's mean.
        assert_eq!(Fusion::Dbsf.normalize(-0.1, &two, 1.0), 0.0);
        assert_eq!(Fusion::parse("Dbsf"), Some(Fusion::Dbsf));
        assert_eq!(Fusion::parse("rrf"), Some(rrf));
        assert_eq!(Fusion::parse("sum"), None);
        assert_eq!(
            rrf,
            Fusion::Rrf(Rrf {
                k: 2,
                weights: None
            })
        );
    }

    #[test]
    fn rrf_rank_scores_follow_qdrant() {
        let rrf = Rrf::default();
        assert_eq!(rrf.rank_score(0, 1.0), 0.5);
        assert!((rrf.rank_score(3, 1.0) - 0.2).abs() < 1e-7);
        // Weight w divides the 1-based rank: second at weight 2 and third at weight 3
        // score like first at weight 1; first at weight 2 scores 1 / 1.5.
        assert_eq!(rrf.rank_score(1, 2.0), rrf.rank_score(0, 1.0));
        assert_eq!(rrf.rank_score(2, 3.0), rrf.rank_score(0, 1.0));
        assert!((rrf.rank_score(0, 2.0) - 1.0 / 1.5).abs() < 1e-7);
        let k60 = Rrf {
            k: 60,
            weights: None,
        };
        assert!((k60.rank_score(0, 3.0) - 1.0 / (1.0 / 3.0 + 59.0)).abs() < 1e-7);
    }

    #[test]
    fn weighted_rrf_sends_a_weight_per_prefetch_list() {
        let queries = [
            query(&[0.5, 0.25], "ERR_CONN_RESET"),
            query(&[0.125, 0.75], "?"),
            query(&[0.25, 0.5], "checkout"),
        ];
        let fusion = Fusion::Rrf(Rrf {
            k: 10,
            weights: Some(RrfWeights {
                dense: 2.0,
                keyword: 0.5,
            }),
        });
        let (body, lists) = hybrid_query_body(&queries, 10, None, fusion);
        use ListKind::{Dense, Keyword};
        assert_eq!(lists, [Dense, Keyword, Dense, Dense, Keyword]);
        assert_eq!(
            body["query"],
            serde_json::json!({"rrf": {"k": 10, "weights": [2.0, 0.5, 2.0, 2.0, 0.5]}})
        );
        let (plain, _) = hybrid_query_body(&queries, 10, None, Fusion::default());
        assert_eq!(body["prefetch"], plain["prefetch"]);
        assert_eq!(plain["query"], serde_json::json!({"rrf": {"k": 2}}));
    }

    #[test]
    fn dbsf_queries_fuse_the_same_prefetches_with_dbsf() {
        let queries = [query(&[0.5, 0.25], "ERR_CONN_RESET")];
        let (rrf, n_rrf) = hybrid_query_body(&queries, 10, None, Fusion::default());
        let (dbsf, n_dbsf) = hybrid_query_body(&queries, 10, None, Fusion::Dbsf);
        assert_eq!(n_rrf, n_dbsf);
        assert_eq!(dbsf["query"], serde_json::json!({"fusion": "dbsf"}));
        assert_eq!(rrf["prefetch"], dbsf["prefetch"]);
        assert_eq!(rrf["limit"], dbsf["limit"]);
    }

    #[tokio::test]
    async fn dbsf_hybrid_search_normalizes_by_lists_or_the_top_point() {
        use wiremock::matchers::{body_json, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let queries = [query(&[0.5, 0.25], "ERR_CONN_RESET")];
        let (body, lists) = hybrid_query_body(&queries, 10, None, Fusion::Dbsf);
        assert_eq!(lists.len(), 2);
        let point = |id: &str, score: f32| {
            serde_json::json!({"id": 1, "version": 1, "score": score,
                "payload": {"Title": "t", "Text": "x", "SourceUri": "u", "Service": "",
                            "Environment": "", "id": id, "Timestamp": null, "Kind": "logs"}})
        };
        Mock::given(method("POST"))
            .and(path("/collections/test/points/query"))
            .and(body_json(body))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": {"points": [point("a", 1.6), point("b", 0.8)]}, "status": "ok"
            })))
            .expect(1)
            .mount(&server)
            .await;
        let mut qd = mock_qdrant(server.uri());
        qd.hybrid.fusion = Fusion::Dbsf;
        let hits = qd.hybrid_search(&queries, 10, None).await.unwrap();
        let scores: Vec<f32> = hits.iter().map(|h| h.score).collect();
        assert!((scores[0] - 0.8).abs() < 1e-6, "{scores:?}");
        assert!((scores[1] - 0.4).abs() < 1e-6, "{scores:?}");
    }

    #[test]
    fn test_qdrant_new_from_env_custom() {
        let _env_lock = lock_env();

        unsafe {
            std::env::set_var("QDRANT_ENDPOINT", "http://custom:6333");
            std::env::set_var("QDRANT_COLLECTION", "custom_collection");
        }

        let qdrant = Qdrant::new_from_env().unwrap();
        assert_eq!(qdrant.endpoint, "http://custom:6333");
        assert_eq!(qdrant.collection, "custom_collection");

        // Cleanup
        unsafe {
            std::env::remove_var("QDRANT_ENDPOINT");
            std::env::remove_var("QDRANT_COLLECTION");
        }
    }

    #[test]
    fn test_payload_from_document() {
        let mut metadata = serde_json::Map::new();
        metadata.insert(
            "custom_field".to_string(),
            serde_json::json!("custom_value"),
        );

        let doc = RagDocument {
            id: "test_id_123".to_string(),
            title: "Test Title".to_string(),
            text: "Test text content".to_string(),
            source_uri: "http://example.com/doc".to_string(),
            kind: SourceKind::Monitor,
            timestamp: Some("2025-01-01T00:00:00Z".to_string()),
            service: "test-service".to_string(),
            environment: "production".to_string(),
            metadata,
        };

        let payload = serde_json::to_value(payload_from(&doc)).unwrap();

        assert_eq!(
            payload.get("Title").unwrap().as_str().unwrap(),
            "Test Title"
        );
        assert_eq!(
            payload.get("Text").unwrap().as_str().unwrap(),
            "Test text content"
        );
        assert_eq!(
            payload.get("SourceUri").unwrap().as_str().unwrap(),
            "http://example.com/doc"
        );
        assert_eq!(
            payload.get("Service").unwrap().as_str().unwrap(),
            "test-service"
        );
        assert_eq!(
            payload.get("Environment").unwrap().as_str().unwrap(),
            "production"
        );
        assert_eq!(payload.get("id").unwrap().as_str().unwrap(), "test_id_123");
        assert_eq!(
            payload.get("Timestamp").unwrap().as_str().unwrap(),
            "2025-01-01T00:00:00Z"
        );

        let metadata = payload.get("Metadata").unwrap().as_object().unwrap();
        assert_eq!(
            metadata.get("custom_field").unwrap().as_str().unwrap(),
            "custom_value"
        );
    }

    #[test]
    fn test_payload_from_document_none_timestamp() {
        let doc = RagDocument {
            id: "test".to_string(),
            title: "Title".to_string(),
            text: "Text".to_string(),
            source_uri: "http://example.com".to_string(),
            kind: SourceKind::Logs,
            timestamp: None,
            service: "svc".to_string(),
            environment: "dev".to_string(),
            metadata: serde_json::Map::new(),
        };

        let payload = serde_json::to_value(payload_from(&doc)).unwrap();
        assert!(payload.get("Timestamp").unwrap().is_null());
    }

    #[test]
    fn hybrid_query_prefetches_dense_and_sparse_per_query_with_the_filter() {
        let filter = serde_json::json!({"must": [{"key": "Service", "match": {"value": "api"}}]});
        let queries = [
            query(&[0.5, 0.25], "ERR_CONN_RESET"),
            query(&[0.125, 0.75], "?"),
        ];
        let (body, lists) = hybrid_query_body(&queries, 10, Some(&filter), Fusion::default());
        // The second query has no tokens, so it only searches the dense vector.
        assert_eq!(lists, [ListKind::Dense, ListKind::Keyword, ListKind::Dense]);
        assert_eq!(
            body,
            serde_json::json!({
                "prefetch": [
                    {"query": [0.5, 0.25], "using": "dense", "limit": 10, "filter": filter},
                    {"query": {"indices": [1695364032u32, 1821864748u32, 2747093051u32, 3560979775u32],
                               "values": [1.0, 1.0, 1.0, 1.0]},
                     "using": "sparse", "limit": 10, "filter": filter},
                    {"query": [0.125, 0.75], "using": "dense", "limit": 10, "filter": filter}
                ],
                "query": {"rrf": {"k": 2}},
                "limit": 10,
                "with_payload": true
            })
        );
        // Without a filter the prefetches carry none.
        let (body, lists) = hybrid_query_body(&queries[..1], 5, None, Fusion::default());
        assert_eq!(lists.len(), 2);
        for p in body["prefetch"].as_array().unwrap() {
            assert!(p.get("filter").is_none(), "{p}");
            assert_eq!(p["limit"], 5);
        }
    }

    #[tokio::test]
    async fn hybrid_search_normalizes_fused_scores_and_restores_every_field() {
        use wiremock::matchers::{body_json, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        let filter = serde_json::json!({"must": [{"key": "Service", "match": {"value": "api"}}]});
        let queries = [
            query(&[0.5, 0.25], "ERR_CONN_RESET"),
            query(&[0.125, 0.75], "?"),
        ];
        let (body, _) = hybrid_query_body(&queries, 10, Some(&filter), Fusion::default());
        let payload = serde_json::json!({
            "Title": "Test Document",
            "Text": "Document content",
            "SourceUri": "http://example.com",
            "Service": "api",
            "Environment": "prod",
            "id": "doc1#c0",
            "Timestamp": "2025-01-01T00:00:00Z",
            "Kind": "sLO",
            "Metadata": {"chunk_of": "doc1", "chunk_index": 0}
        });
        Mock::given(method("POST"))
            .and(path("/collections/test/points/query"))
            .and(body_json(body))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": {"points": [
                    // First in all three lists: 3 × 1/2.
                    {"id": "21aca85b-31bc-5c36-bd5f-32403cb008d2", "version": 3, "score": 1.5,
                     "payload": payload},
                    // Second in one list: 1/3.
                    {"id": 7, "version": 1, "score": 0.333_333_34,
                     "payload": {"Title": "t", "Text": "x", "SourceUri": "u", "Service": "",
                                 "Environment": "", "id": "other", "Timestamp": null,
                                 "Kind": "logs"}}
                ]},
                "status": "ok"
            })))
            .expect(1)
            .mount(&mock_server)
            .await;

        let hits = mock_qdrant(mock_server.uri())
            .hybrid_search(&queries, 10, Some(filter.clone()))
            .await
            .unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].score, 1.0);
        assert!(
            (hits[1].score - 2.0 / 9.0).abs() < 1e-6,
            "{}",
            hits[1].score
        );
        let doc = &hits[0].doc;
        assert_eq!(doc.id, "doc1#c0");
        assert_eq!(doc.parent_id(), "doc1");
        assert_eq!(doc.title, "Test Document");
        assert_eq!(doc.text, "Document content");
        assert_eq!(doc.source_uri, "http://example.com");
        assert_eq!(doc.kind, SourceKind::SLO);
        assert_eq!(doc.timestamp.as_deref(), Some("2025-01-01T00:00:00Z"));
        assert_eq!(
            (doc.service.as_str(), doc.environment.as_str()),
            ("api", "prod")
        );
        // No query at all is a caller bug, never an empty result.
        assert!(
            mock_qdrant(mock_server.uri())
                .hybrid_search(&[], 10, None)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn upsert_waits_and_sends_uuid_points_with_the_storage_payload() {
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/collections/test/points"))
            .and(query_param("wait", "true"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": {"operation_id": 1, "status": "completed"}
            })))
            .expect(1)
            .mount(&mock_server)
            .await;

        let doc = contract_document();
        let point = QPoint::from_document(&doc, vec![0.1, 0.2, 0.3]);
        // The sparse vector as sent: f32 values printed, then read back.
        let sparse: serde_json::Value = serde_json::from_str(
            &serde_json::to_string(&crate::sparse::document_vector(&embedding_input(&doc)))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(sparse["indices"].as_array().unwrap().len(), 9);
        mock_qdrant(mock_server.uri())
            .upsert(vec![point])
            .await
            .unwrap();

        let reqs = mock_server.received_requests().await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&reqs[0].body).unwrap();
        assert_eq!(
            body,
            serde_json::json!({"points": [{
                "id": "21aca85b-31bc-5c36-bd5f-32403cb008d2",
                "vector": {
                    "dense": [0.1, 0.2, 0.3],
                    "sparse": sparse
                },
                "payload": {
                    "id": "monitor_123#c0",
                    "Title": "Återkommande fel",
                    "Text": "Connection failed",
                    "SourceUri": "https://app.datadoghq.eu/monitors/123",
                    "Kind": "monitor",
                    "Timestamp": null,
                    "Service": "payments",
                    "Environment": "prod",
                    "Metadata": {"chunk_of": "monitor_123", "chunk_index": 0}
                }
            }]})
        );
    }

    #[tokio::test]
    async fn search_and_upsert_failures_are_typed_by_stage() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/collections/test/points/query"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&mock_server)
            .await;
        Mock::given(method("PUT"))
            .and(path("/collections/test/points"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&mock_server)
            .await;
        let qdrant = mock_qdrant(mock_server.uri());

        // A missing collection is a retrieval failure, not "no evidence".
        let err = qdrant
            .hybrid_search(&[query(&[0.1], "x")], 10, None)
            .await
            .unwrap_err();
        assert!(matches!(err, RagError::RetrievalFailed { .. }), "{err:?}");

        let point = QPoint::from_document(&contract_document(), vec![0.1]);
        let err = qdrant.upsert(vec![point]).await.unwrap_err();
        assert!(
            matches!(
                err,
                RagError::UpstreamUnavailable {
                    stage: Stage::Indexing,
                    ..
                }
            ),
            "{err:?}"
        );
    }

    fn mock_qdrant(uri: String) -> Qdrant {
        let mut qdrant = Qdrant::new(uri, "test".into());
        qdrant.retry = crate::resilience::RetryPolicy::none();
        qdrant
    }

    #[test]
    fn bookkeeping_fields_are_optional_and_omitted_when_unset() {
        let doc = contract_document();
        let value = serde_json::to_value(payload_from(&doc)).unwrap();
        for key in [CONTENT_HASH_KEY, CHUNK_COUNT_KEY, SYNC_ID_KEY] {
            assert!(value.get(key).is_none(), "{key}");
        }
        // Legacy payloads without the keys still decode.
        let legacy: QdrantPayload = serde_json::from_value(value).unwrap();
        assert_eq!(legacy.content_hash, None);

        let mut point = QPoint::from_document(&doc, vec![1.0]);
        point.payload.content_hash = Some("abc".into());
        point.payload.chunk_count = Some(2);
        point.payload.sync_id = Some("run-1".into());
        let value = serde_json::to_value(&point.payload).unwrap();
        assert_eq!(value["ContentHash"], "abc");
        assert_eq!(value["ChunkCount"], 2);
        assert_eq!(value["SyncId"], "run-1");
        // Bookkeeping never reaches the public document.
        let restored = RagDocument::from(serde_json::from_value::<QdrantPayload>(value).unwrap());
        assert_eq!(
            serde_json::to_value(restored).unwrap(),
            serde_json::to_value(doc).unwrap()
        );
    }

    #[tokio::test]
    async fn retrieve_states_posts_ids_and_reads_bookkeeping() {
        use wiremock::matchers::{body_json, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let known = point_id("monitor_1#c0");
        let legacy = point_id("monitor_1#c1");
        let missing = point_id("monitor_1#c2");
        Mock::given(method("POST"))
            .and(path("/collections/test/points"))
            .and(body_json(serde_json::json!({
                "ids": [known, legacy, missing],
                "with_payload": ["ContentHash", "ChunkCount"],
                "with_vector": false
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": [
                    {"id": known, "payload": {"ContentHash": "h", "ChunkCount": 2}},
                    {"id": legacy, "payload": {}}
                ],
                "status": "ok"
            })))
            .expect(1)
            .mount(&server)
            .await;

        let states = mock_qdrant(server.uri())
            .retrieve_states(&[known, legacy, missing])
            .await
            .unwrap();
        assert_eq!(
            states,
            [
                StoredPointState {
                    id: known,
                    content_hash: Some("h".into()),
                    chunk_count: Some(2)
                },
                StoredPointState {
                    id: legacy,
                    content_hash: None,
                    chunk_count: None
                }
            ]
        );
        // No request for an empty ID list.
        assert!(
            mock_qdrant(server.uri())
                .retrieve_states(&[])
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn retrieve_metadata_posts_ids_and_reads_metadata() {
        use wiremock::matchers::{body_json, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let pattern = point_id("logpattern_1#c0");
        let bare = point_id("log_2#c0");
        let missing = point_id("logpattern_3#c0");
        Mock::given(method("POST"))
            .and(path("/collections/test/points"))
            .and(body_json(serde_json::json!({
                "ids": [pattern, bare, missing],
                "with_payload": ["Metadata"],
                "with_vector": false
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": [
                    {"id": pattern, "payload": {"Metadata": {"count": 3, "pattern": "p #"}}},
                    {"id": bare, "payload": {}}
                ],
                "status": "ok"
            })))
            .expect(1)
            .mount(&server)
            .await;

        let found = mock_qdrant(server.uri())
            .retrieve_metadata(&[pattern, bare, missing])
            .await
            .unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].0, pattern);
        assert_eq!(found[0].1["count"], 3);
        assert_eq!(found[1], (bare, serde_json::Map::new()));
        assert!(
            mock_qdrant(server.uri())
                .retrieve_metadata(&[])
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn set_payload_posts_points_and_payload() {
        use wiremock::matchers::{body_json, method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let id = point_id("slo_1#c0");
        Mock::given(method("POST"))
            .and(path("/collections/test/points/payload"))
            .and(query_param("wait", "true"))
            .and(body_json(serde_json::json!({
                "payload": {"SyncId": "run-1"},
                "points": [id]
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": {"operation_id": 1, "status": "completed"}, "status": "ok"
            })))
            .expect(1)
            .mount(&server)
            .await;

        mock_qdrant(server.uri())
            .set_payload(&[id], serde_json::json!({"SyncId": "run-1"}))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn delete_by_filter_posts_surplus_filter() {
        use wiremock::matchers::{body_json, method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/collections/test/points/delete"))
            .and(query_param("wait", "true"))
            .and(body_json(serde_json::json!({"filter": {"must": [
                {"key": "Kind", "match": {"value": "monitor"}},
                {"key": "Metadata.chunk_of", "match": {"value": "monitor_1"}},
                {"key": "Metadata.chunk_index", "range": {"gte": 2}}
            ]}})))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": {"operation_id": 2, "status": "completed"}, "status": "ok"
            })))
            .expect(1)
            .mount(&server)
            .await;

        mock_qdrant(server.uri())
            .delete_by_filter(surplus_chunks_filter(&SourceKind::Monitor, "monitor_1", 2))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn count_posts_exact_unsynced_filter() {
        use wiremock::matchers::{body_json, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/collections/test/points/count"))
            .and(body_json(serde_json::json!({
                "filter": {
                    "must": [{"key": "Kind", "match": {"value": "sLO"}}],
                    "must_not": [{"key": "SyncId", "match": {"value": "run-1"}}]
                },
                "exact": true
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": {"count": 7}, "status": "ok"
            })))
            .expect(1)
            .mount(&server)
            .await;

        let count = mock_qdrant(server.uri())
            .count(unsynced_filter(&SourceKind::SLO, "run-1"))
            .await
            .unwrap();
        assert_eq!(count, 7);
    }

    #[tokio::test]
    async fn indexing_calls_fail_on_error_status() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        let qdrant = mock_qdrant(server.uri());
        let id = point_id("x#c0");
        assert!(qdrant.retrieve_states(&[id]).await.is_err());
        assert!(
            qdrant
                .set_payload(&[id], serde_json::json!({}))
                .await
                .is_err()
        );
        assert!(
            qdrant
                .delete_by_filter(unsynced_filter(&SourceKind::Monitor, "r"))
                .await
                .is_err()
        );
        assert!(
            qdrant
                .count(unsynced_filter(&SourceKind::Monitor, "r"))
                .await
                .is_err()
        );
    }
}
