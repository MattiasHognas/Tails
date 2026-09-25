use crate::chunk::embedding_input;
use crate::domain::{Hit, RagDocument, SourceKind};
use crate::error::{RagError, Stage, UpstreamError};
use crate::resilience::{HttpConfig, RetryPolicy, send_with_retry};
use crate::sparse::{SparseVector, document_vector};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

/// Name of the dense (embedding) vector of every point.
pub const DENSE_VECTOR: &str = "dense";
/// Name of the sparse keyword vector of every point (see [`crate::sparse`]).
pub const SPARSE_VECTOR: &str = "sparse";
/// `k` of reciprocal rank fusion: a point at 0-based rank `r` of a prefetch list scores
/// `1 / (k + r)`. 2 is Qdrant's default, sent explicitly so the fused score (and its
/// normalization in [`Qdrant::hybrid_search`]) cannot change with a Qdrant upgrade.
pub const RRF_K: u32 = 2;

#[derive(Debug, Clone)]
pub struct Qdrant {
    pub endpoint: String,
    pub collection: String,
    pub http: reqwest::Client,
    pub retry: RetryPolicy,
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

/// Body of the hybrid `POST /collections/{c}/points/query` and the number of prefetch
/// lists it fuses: per query a dense and (unless it has no tokens) a sparse prefetch,
/// each restricted by `filter` and returning up to `limit` points, fused with reciprocal
/// rank fusion ([`RRF_K`]).
pub fn hybrid_query_body(
    queries: &[SearchQuery],
    limit: usize,
    filter: Option<&serde_json::Value>,
) -> (serde_json::Value, usize) {
    let prefetch_of = |query: serde_json::Value, using: &str| {
        let mut p = json!({"query": query, "using": using, "limit": limit});
        if let Some(f) = filter {
            p["filter"] = f.clone();
        }
        p
    };
    let mut prefetch = vec![];
    for q in queries {
        prefetch.push(prefetch_of(json!(q.dense), DENSE_VECTOR));
        if !q.sparse.is_empty() {
            prefetch.push(prefetch_of(json!(q.sparse), SPARSE_VECTOR));
        }
    }
    let lists = prefetch.len();
    let body = json!({
        "prefetch": prefetch,
        "query": {"rrf": {"k": RRF_K}},
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
        }
    }

    pub fn new_from_env() -> Result<Self> {
        let mut qdrant = Self::new(
            std::env::var("QDRANT_ENDPOINT").unwrap_or_else(|_| "http://localhost:6333".into()),
            std::env::var("QDRANT_COLLECTION").unwrap_or_else(|_| "datadog_rag".into()),
        );
        qdrant.http = HttpConfig::from_env().build_client();
        qdrant.retry = RetryPolicy::from_env();
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
    /// `filter`, fused by Qdrant with reciprocal rank fusion (see [`hybrid_query_body`]).
    ///
    /// Returns at most `limit` hits. A hit's score is its fused RRF score divided by the
    /// best possible one (first in every prefetch list), so it lies in (0, 1]: 1 means
    /// ranked first by every search, 0.667 second by all, 0.5 first by half of them or
    /// third by all (with [`RRF_K`] = 2). Only ranks count, not the raw similarities, so
    /// scores of different questions are comparable and the reranker's kind priors and
    /// recency weight ([`crate::reranker::recency_weight`]) scale them like before.
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
        let (req, lists) = hybrid_query_body(queries, limit, filter.as_ref());
        if lists == 0 {
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
        let best = lists as f32 / RRF_K as f32;
        Ok(v.result
            .points
            .into_iter()
            .map(|it| Hit {
                doc: it.payload.into(),
                score: (it.score / best).min(1.0),
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
        let (body, lists) = hybrid_query_body(&queries, 10, Some(&filter));
        // The second query has no tokens, so it only searches the dense vector.
        assert_eq!(lists, 3);
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
        let (body, lists) = hybrid_query_body(&queries[..1], 5, None);
        assert_eq!(lists, 2);
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
        let (body, _) = hybrid_query_body(&queries, 10, Some(&filter));
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
