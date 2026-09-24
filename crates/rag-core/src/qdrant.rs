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
}

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
        // This namespace is part of the persisted ID contract: do not change it.
        let namespace = Uuid::new_v5(
            &Uuid::NAMESPACE_URL,
            b"https://github.com/MattiasHognas/Tails",
        );
        Self {
            id: Uuid::new_v5(&namespace, doc.id.as_bytes()),
            vector,
            payload: payload_from(doc),
        }
    }
}

pub fn payload_from(doc: &RagDocument) -> QdrantPayload {
    QdrantPayload::from(doc)
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
}
