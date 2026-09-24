use crate::domain::{Hit, RagDocument, SourceKind};
use crate::error::{RagError, Stage, UpstreamError};
use crate::resilience::{HttpConfig, RetryPolicy, send_with_retry};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

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
    pub vector: Vec<f32>,
    pub payload: QdrantPayload,
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
    pub fn from_document(doc: &RagDocument, vector: Vec<f32>) -> Self {
        Self {
            id: point_id(&doc.id),
            vector,
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

    pub async fn search(
        &self,
        vector: Vec<f32>,
        limit: usize,
        filter: Option<serde_json::Value>,
    ) -> Result<Vec<Hit>, RagError> {
        #[derive(Serialize)]
        struct Req<'a> {
            vector: &'a [f32],
            limit: usize,
            with_payload: bool,
            filter: Option<serde_json::Value>,
        }
        #[derive(Deserialize)]
        struct Resp {
            result: Vec<Item>,
        }
        #[derive(Deserialize)]
        struct Item {
            score: f32,
            payload: QdrantPayload,
            #[allow(dead_code)]
            id: serde_json::Value,
        }
        let url = format!(
            "{}/collections/{}/points/search",
            self.endpoint, self.collection
        );
        let req = Req {
            vector: &vector,
            limit,
            with_payload: true,
            filter,
        };
        let r = send_with_retry(&self.retry, "qdrant search", || {
            self.http.post(&url).json(&req)
        })
        .await
        .map_err(|f| RagError::upstream(Stage::Retrieval, f))?;
        // A malformed payload is a retrieval failure, never "no evidence".
        let v: Resp = r
            .json()
            .await
            .map_err(|e| RagError::failed(Stage::Retrieval, UpstreamError::from_reqwest(e)))?;
        Ok(v.result
            .into_iter()
            .map(|it| Hit {
                doc: it.payload.into(),
                score: it.score,
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
            .and(path("/collections/test/points/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": [{"id": 1, "score": 0.9, "payload": payload}]
            })))
            .mount(&server)
            .await;
        let qdrant = Qdrant::new(server.uri(), "test".into());
        assert!(qdrant.search(vec![1.0], 10, None).await.is_err());
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
    fn test_url_formatting() {
        let qdrant = Qdrant {
            endpoint: "http://localhost:6333".to_string(),
            collection: "test_collection".to_string(),
            http: reqwest::Client::new(),
            retry: crate::resilience::RetryPolicy::none(),
        };

        let upsert_url = format!(
            "{}/collections/{}/points?wait=true",
            qdrant.endpoint, qdrant.collection
        );
        assert_eq!(
            upsert_url,
            "http://localhost:6333/collections/test_collection/points?wait=true"
        );

        let search_url = format!(
            "{}/collections/{}/points/search",
            qdrant.endpoint, qdrant.collection
        );
        assert_eq!(
            search_url,
            "http://localhost:6333/collections/test_collection/points/search"
        );
    }

    #[tokio::test]
    async fn test_search_with_mock_server() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/collections/test/points/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": [{
                    "id": "point1",
                    "score": 0.95,
                    "payload": {
                        "Title": "Test Document",
                        "Text": "Document content",
                        "SourceUri": "http://example.com",
                        "Service": "api",
                        "Environment": "prod",
                        "id": "doc1",
                        "Timestamp": "2025-01-01T00:00:00Z",
                        "Kind": "logs"
                    }
                }]
            })))
            .mount(&mock_server)
            .await;

        let qdrant = Qdrant {
            endpoint: mock_server.uri(),
            collection: "test".to_string(),
            http: reqwest::Client::new(),
            retry: crate::resilience::RetryPolicy::none(),
        };

        let vector = vec![0.1, 0.2, 0.3];
        let result = qdrant.search(vector, 10, None).await;
        assert!(result.is_ok(), "Error: {:?}", result.err());
        let hits = result.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].doc.id, "doc1");
    }

    #[tokio::test]
    async fn test_upsert_with_mock_server() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;

        Mock::given(method("PUT"))
            .and(path("/collections/test/points"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "result": {
                    "operation_id": 1,
                    "status": "completed"
                }
            })))
            .mount(&mock_server)
            .await;

        let qdrant = Qdrant {
            endpoint: mock_server.uri(),
            collection: "test".to_string(),
            http: reqwest::Client::new(),
            retry: crate::resilience::RetryPolicy::none(),
        };

        let doc = RagDocument {
            id: "test123".to_string(),
            title: "Test".to_string(),
            text: "Content".to_string(),
            source_uri: "http://example.com".to_string(),
            kind: SourceKind::Logs,
            timestamp: Some("2025-01-01T00:00:00Z".to_string()),
            service: "api".to_string(),
            environment: "prod".to_string(),
            metadata: serde_json::Map::new(),
        };

        let point = QPoint::from_document(&doc, vec![0.1, 0.2, 0.3]);

        let result = qdrant.upsert(vec![point]).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_search_error_handling() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/collections/test/points/search"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&mock_server)
            .await;

        let qdrant = Qdrant {
            endpoint: mock_server.uri(),
            collection: "test".to_string(),
            http: reqwest::Client::new(),
            retry: crate::resilience::RetryPolicy::none(),
        };

        let vector = vec![0.1, 0.2, 0.3];
        let result = qdrant.search(vector, 10, None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_upsert_error_handling() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;

        Mock::given(method("PUT"))
            .and(path("/collections/test/points"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&mock_server)
            .await;

        let qdrant = Qdrant {
            endpoint: mock_server.uri(),
            collection: "test".to_string(),
            http: reqwest::Client::new(),
            retry: crate::resilience::RetryPolicy::none(),
        };

        let doc = RagDocument {
            id: "test123".to_string(),
            title: "Test".to_string(),
            text: "Content".to_string(),
            source_uri: "http://example.com".to_string(),
            kind: SourceKind::Logs,
            timestamp: Some("2025-01-01T00:00:00Z".to_string()),
            service: "api".to_string(),
            environment: "prod".to_string(),
            metadata: serde_json::Map::new(),
        };

        let point = QPoint::from_document(&doc, vec![0.1, 0.2, 0.3]);

        let result = qdrant.upsert(vec![point]).await;
        assert!(result.is_err());
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
