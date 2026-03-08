use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SourceKind {
    Logs,
    Metrics,
    Monitor,
    Incident,
    Dashboard,
    SLO,
    Git,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RagDocument {
    pub id: String,
    pub title: String,
    pub text: String,
    pub source_uri: String,
    pub kind: SourceKind,
    pub timestamp: Option<String>, // ISO-8601
    pub service: String,
    pub environment: String,
    pub metadata: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    pub doc: RagDocument,
    pub score: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_source_kind_serialization() {
        let kinds = vec![
            SourceKind::Logs,
            SourceKind::Metrics,
            SourceKind::Monitor,
            SourceKind::Incident,
            SourceKind::Dashboard,
            SourceKind::SLO,
            SourceKind::Git,
        ];

        for kind in kinds {
            let json = serde_json::to_string(&kind).unwrap();
            let deserialized: SourceKind = serde_json::from_str(&json).unwrap();
            assert_eq!(kind, deserialized);
        }
    }

    #[test]
    fn test_source_kind_camel_case() {
        let json = r#""logs""#;
        let kind: SourceKind = serde_json::from_str(json).unwrap();
        assert_eq!(kind, SourceKind::Logs);

        let serialized = serde_json::to_string(&SourceKind::Monitor).unwrap();
        assert_eq!(serialized, r#""monitor""#);
    }

    #[test]
    fn test_rag_document_serialization() {
        let mut metadata = serde_json::Map::new();
        metadata.insert("key1".to_string(), serde_json::json!("value1"));

        let doc = RagDocument {
            id: "test123".to_string(),
            title: "Test Document".to_string(),
            text: "This is a test document".to_string(),
            source_uri: "http://example.com/doc".to_string(),
            kind: SourceKind::Logs,
            timestamp: Some("2025-01-01T00:00:00Z".to_string()),
            service: "api-service".to_string(),
            environment: "production".to_string(),
            metadata,
        };

        let json = serde_json::to_string(&doc).unwrap();
        let deserialized: RagDocument = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.id, "test123");
        assert_eq!(deserialized.title, "Test Document");
        assert_eq!(deserialized.service, "api-service");
        assert_eq!(deserialized.environment, "production");
    }

    #[test]
    fn test_rag_document_optional_timestamp() {
        let doc = RagDocument {
            id: "test".to_string(),
            title: "Test".to_string(),
            text: "Test".to_string(),
            source_uri: "http://example.com".to_string(),
            kind: SourceKind::Metrics,
            timestamp: None,
            service: "svc".to_string(),
            environment: "dev".to_string(),
            metadata: serde_json::Map::new(),
        };

        let json = serde_json::to_string(&doc).unwrap();
        let deserialized: RagDocument = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.timestamp, None);
    }

    #[test]
    fn test_hit_serialization() {
        let doc = RagDocument {
            id: "hit_test".to_string(),
            title: "Hit Test".to_string(),
            text: "This is a hit test".to_string(),
            source_uri: "http://example.com".to_string(),
            kind: SourceKind::Monitor,
            timestamp: Some("2025-01-01T12:00:00Z".to_string()),
            service: "monitor-svc".to_string(),
            environment: "staging".to_string(),
            metadata: serde_json::Map::new(),
        };

        let hit = Hit { doc, score: 0.95 };

        let json = serde_json::to_string(&hit).unwrap();
        let deserialized: Hit = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.score, 0.95);
        assert_eq!(deserialized.doc.id, "hit_test");
    }

    #[test]
    fn test_rag_document_camel_case_fields() {
        let json = r#"{
            "id": "test",
            "title": "Test",
            "text": "Text",
            "sourceUri": "http://example.com",
            "kind": "logs",
            "timestamp": null,
            "service": "svc",
            "environment": "env",
            "metadata": {}
        }"#;

        let doc: RagDocument = serde_json::from_str(json).unwrap();
        assert_eq!(doc.source_uri, "http://example.com");

        let serialized = serde_json::to_value(&doc).unwrap();
        assert!(serialized.get("sourceUri").is_some());
    }
}
