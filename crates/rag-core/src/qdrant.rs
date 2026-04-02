use crate::domain::{Hit, RagDocument};
use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct Qdrant {
    pub endpoint: String,
    pub collection: String,
    pub http: reqwest::Client,
}

#[derive(Debug, Clone, Serialize)]
pub struct QPoint {
    pub id: String,
    pub vector: Vec<f32>,
    pub payload: serde_json::Value,
}

pub fn payload_from(doc: &RagDocument) -> serde_json::Value {
    serde_json::json!({
      "Title": doc.title,
      "Text": doc.text,
      "SourceUri": doc.source_uri,
      "Kind": doc.kind,
      "Timestamp": doc.timestamp,
      "Service": doc.service,
      "Environment": doc.environment,
      "Metadata": doc.metadata,
      "id": doc.id
    })
}

impl Qdrant {
    pub fn new(endpoint: String, collection: String) -> Self {
        Self {
            endpoint,
            collection,
            http: reqwest::Client::new(),
        }
    }

    pub fn new_from_env() -> Result<Self> {
        Ok(Self::new(
            std::env::var("QDRANT_ENDPOINT").unwrap_or_else(|_| "http://localhost:6333".into()),
            std::env::var("QDRANT_COLLECTION").unwrap_or_else(|_| "datadog_rag".into()),
        ))
    }

    pub async fn upsert(&self, points: Vec<QPoint>) -> Result<()> {
        #[derive(Serialize)]
        struct Req {
            points: Vec<QPoint>,
        }
        let url = format!(
            "{}/collections/{}/points?wait=true",
            self.endpoint, self.collection
        );
        let r = self.http.put(url).json(&Req { points }).send().await?;
        if !r.status().is_success() {
            return Err(anyhow!("qdrant upsert status {}", r.status()));
        }
        Ok(())
    }

    pub async fn search(
        &self,
        vector: Vec<f32>,
        limit: usize,
        filter: Option<serde_json::Value>,
    ) -> Result<Vec<Hit>> {
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
            payload: serde_json::Value,
            #[allow(dead_code)]
            id: serde_json::Value,
        }
        let url = format!(
            "{}/collections/{}/points/search",
            self.endpoint, self.collection
        );
        let r = self
            .http
            .post(url)
            .json(&Req {
                vector: &vector,
                limit,
                with_payload: true,
                filter,
            })
            .send()
            .await?;
        if !r.status().is_success() {
            return Err(anyhow!("qdrant search status {}", r.status()));
        }
        let v: Resp = r.json().await?;
        let mut hits = Vec::new();
        for it in v.result {
            let p = it.payload;
            let doc = RagDocument {
                id: p
                    .get("id")
                    .and_then(|x| x.as_str())
                    .unwrap_or_default()
                    .to_string(),
                title: p
                    .get("Title")
                    .and_then(|x| x.as_str())
                    .unwrap_or_default()
                    .to_string(),
                text: p
                    .get("Text")
                    .and_then(|x| x.as_str())
                    .unwrap_or_default()
                    .to_string(),
                source_uri: p
                    .get("SourceUri")
                    .and_then(|x| x.as_str())
                    .unwrap_or_default()
                    .to_string(),
                kind: serde_json::from_value(
                    p.get("Kind")
                        .cloned()
                        .unwrap_or_else(|| serde_json::Value::String("Logs".into())),
                )?,
                timestamp: p
                    .get("Timestamp")
                    .and_then(|x| x.as_str())
                    .map(|s| s.to_string()),
                service: p
                    .get("Service")
                    .and_then(|x| x.as_str())
                    .unwrap_or_default()
                    .to_string(),
                environment: p
                    .get("Environment")
                    .and_then(|x| x.as_str())
                    .unwrap_or_default()
                    .to_string(),
                metadata: p
                    .get("Metadata")
                    .and_then(|x| x.as_object())
                    .cloned()
                    .unwrap_or_default(),
            };
            hits.push(Hit {
                doc,
                score: it.score,
            });
        }
        Ok(hits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::SourceKind;
    use crate::test_support::lock_env;

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

        let payload = payload_from(&doc);

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

        let payload = payload_from(&doc);
        assert!(payload.get("Timestamp").unwrap().is_null());
    }

    #[test]
    fn test_qpoint_structure() {
        let point = QPoint {
            id: "point123".to_string(),
            vector: vec![0.1, 0.2, 0.3],
            payload: serde_json::json!({"key": "value"}),
        };

        assert_eq!(point.id, "point123");
        assert_eq!(point.vector.len(), 3);
        assert_eq!(point.payload.get("key").unwrap().as_str().unwrap(), "value");
    }

    #[test]
    fn test_url_formatting() {
        let qdrant = Qdrant {
            endpoint: "http://localhost:6333".to_string(),
            collection: "test_collection".to_string(),
            http: reqwest::Client::new(),
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

        let point = QPoint {
            id: doc.id.clone(),
            vector: vec![0.1, 0.2, 0.3],
            payload: payload_from(&doc),
        };

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

        let point = QPoint {
            id: doc.id.clone(),
            vector: vec![0.1, 0.2, 0.3],
            payload: payload_from(&doc),
        };

        let result = qdrant.upsert(vec![point]).await;
        assert!(result.is_err());
    }
}
