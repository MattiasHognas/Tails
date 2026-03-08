use anyhow::Result;
use rag_core::{
    chunk::chunk,
    datadog::Datadog,
    domain::RagDocument,
    openai::OpenAiClient,
    qdrant::{QPoint, Qdrant},
};
use serde_json::json;

/// Maximum characters per chunk
const CHUNK_SIZE: usize = 1800;
/// Characters of overlap between consecutive chunks
const CHUNK_OVERLAP: usize = 200;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let watermark_path =
        std::env::var("INDEXER_WATERMARK").unwrap_or_else(|_| "./watermark.json".to_string());
    let lookback_minutes = std::env::var("INDEXER_LOOKBACK_MINUTES")
        .unwrap_or_else(|_| "90".to_string())
        .parse::<i64>()
        .unwrap_or(90);

    let dd = Datadog::new_from_env()?;
    let oa = OpenAiClient::new_from_env()?;
    let qd = Qdrant::new_from_env()?;

    let (from_iso, to_iso) = window(&watermark_path, lookback_minutes).await?;

    tracing::info!("Fetching Datadog data from {} to {}", from_iso, to_iso);

    let mut docs = Vec::new();
    docs.extend(dd.get_monitors().await?);
    docs.extend(dd.list_dashboards().await?);
    docs.extend(dd.list_slos().await?);
    docs.extend(dd.list_metrics(&from_iso, &to_iso).await?);
    docs.extend(dd.get_incidents(&from_iso, &to_iso).await?);
    docs.extend(dd.search_logs(&from_iso, &to_iso).await?);

    tracing::info!("Chunking {} documents", docs.len());
    let chunks = docs
        .iter()
        .flat_map(|d| chunk(CHUNK_SIZE, CHUNK_OVERLAP, d))
        .collect::<Vec<_>>();

    tracing::info!("Embedding and upserting {} chunks", chunks.len());
    let mut batch = Vec::new();
    for c in chunks {
        let emb = oa.embed(&c.text).await?;
        batch.push(QPoint {
            id: c.id.clone(),
            vector: emb,
            payload: payload_from(&c),
        });
        if batch.len() >= 64 {
            qd.upsert(std::mem::take(&mut batch)).await?;
        }
    }
    if !batch.is_empty() {
        qd.upsert(batch).await?;
    }

    save_watermark(&watermark_path, &to_iso).await?;
    tracing::info!("Indexing complete");
    Ok(())
}

async fn window(watermark_path: &str, lookback_minutes: i64) -> Result<(String, String)> {
    let now = chrono::Utc::now();
    let to_iso = now.to_rfc3339();

    let from = if let Ok(content) = tokio::fs::read_to_string(watermark_path).await {
        if let Ok(ts) = chrono::DateTime::parse_from_rfc3339(content.trim()) {
            ts.with_timezone(&chrono::Utc)
        } else {
            now - chrono::Duration::minutes(lookback_minutes)
        }
    } else {
        now - chrono::Duration::minutes(lookback_minutes)
    };

    let from_iso = from.to_rfc3339();
    Ok((from_iso, to_iso))
}

async fn save_watermark(path: &str, timestamp: &str) -> Result<()> {
    tokio::fs::write(path, timestamp).await?;
    Ok(())
}

fn payload_from(doc: &RagDocument) -> serde_json::Value {
    json!({
        "title": doc.title,
        "text": doc.text,
        "source_uri": doc.source_uri,
        "kind": doc.kind,
        "timestamp": doc.timestamp,
        "Service": doc.service,
        "Environment": doc.environment,
        "metadata": doc.metadata,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rag_core::domain::SourceKind;

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
        let payload = payload_from(&doc);

        assert_eq!(payload["title"], "Test Document");
        assert_eq!(payload["text"], "This is test content");
        assert_eq!(payload["source_uri"], "http://example.com/test");
        assert_eq!(payload["Service"], "test-service");
        assert_eq!(payload["Environment"], "production");
    }

    #[test]
    fn test_payload_from_with_timestamp() {
        let doc = create_test_doc();
        let payload = payload_from(&doc);

        assert_eq!(payload["timestamp"], "2025-01-01T00:00:00Z");
    }

    #[test]
    fn test_payload_from_without_timestamp() {
        let mut doc = create_test_doc();
        doc.timestamp = None;
        let payload = payload_from(&doc);

        assert!(payload["timestamp"].is_null());
    }

    #[test]
    fn test_payload_from_preserves_kind() {
        let mut doc = create_test_doc();
        doc.kind = SourceKind::Incident;
        let payload = payload_from(&doc);

        assert_eq!(payload["kind"], serde_json::json!(SourceKind::Incident));
    }

    #[test]
    fn test_payload_from_with_metadata() {
        let mut doc = create_test_doc();
        let mut metadata = serde_json::Map::new();
        metadata.insert("severity".to_string(), serde_json::json!("SEV-1"));
        metadata.insert("status".to_string(), serde_json::json!("active"));
        doc.metadata = metadata;

        let payload = payload_from(&doc);

        assert_eq!(payload["metadata"]["severity"], "SEV-1");
        assert_eq!(payload["metadata"]["status"], "active");
    }

    #[test]
    fn test_payload_from_empty_metadata() {
        let doc = create_test_doc();
        let payload = payload_from(&doc);

        assert!(payload["metadata"].is_object());
        assert_eq!(payload["metadata"].as_object().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn test_save_watermark_creates_file() {
        let temp_dir = std::env::temp_dir();
        let test_file = temp_dir.join("test_watermark.txt");
        let path_str = test_file.to_str().unwrap();

        let timestamp = "2025-01-01T12:00:00Z";
        let result = save_watermark(path_str, timestamp).await;

        assert!(result.is_ok());

        // Verify file was created and contains correct content
        let content = tokio::fs::read_to_string(path_str).await.unwrap();
        assert_eq!(content, timestamp);

        // Cleanup
        let _ = tokio::fs::remove_file(path_str).await;
    }

    #[tokio::test]
    async fn test_save_watermark_overwrites_existing() {
        let temp_dir = std::env::temp_dir();
        let test_file = temp_dir.join("test_watermark_overwrite.txt");
        let path_str = test_file.to_str().unwrap();

        // Write initial content
        save_watermark(path_str, "2025-01-01T00:00:00Z")
            .await
            .unwrap();

        // Overwrite with new content
        let new_timestamp = "2025-01-02T00:00:00Z";
        let result = save_watermark(path_str, new_timestamp).await;

        assert!(result.is_ok());

        // Verify new content
        let content = tokio::fs::read_to_string(path_str).await.unwrap();
        assert_eq!(content, new_timestamp);

        // Cleanup
        let _ = tokio::fs::remove_file(path_str).await;
    }

    #[tokio::test]
    async fn test_window_with_valid_watermark() {
        let temp_dir = std::env::temp_dir();
        let test_file = temp_dir.join("test_window_valid.txt");
        let path_str = test_file.to_str().unwrap();

        // Create a watermark file with a valid timestamp
        let watermark_time = "2025-01-01T00:00:00Z";
        tokio::fs::write(path_str, watermark_time).await.unwrap();

        let result = window(path_str, 90).await;
        assert!(result.is_ok());

        let (from_iso, to_iso) = result.unwrap();

        // from should parse to the same time as watermark (allowing for format differences)
        let from_parsed = chrono::DateTime::parse_from_rfc3339(&from_iso).unwrap();
        let watermark_parsed = chrono::DateTime::parse_from_rfc3339(watermark_time).unwrap();
        assert_eq!(from_parsed, watermark_parsed);

        // to should be a valid RFC3339 timestamp
        assert!(chrono::DateTime::parse_from_rfc3339(&to_iso).is_ok());

        // Cleanup
        let _ = tokio::fs::remove_file(path_str).await;
    }

    #[tokio::test]
    async fn test_window_with_invalid_watermark() {
        let temp_dir = std::env::temp_dir();
        let test_file = temp_dir.join("test_window_invalid.txt");
        let path_str = test_file.to_str().unwrap();

        // Create a watermark file with invalid content
        tokio::fs::write(path_str, "invalid timestamp")
            .await
            .unwrap();

        let result = window(path_str, 90).await;
        assert!(result.is_ok());

        let (from_iso, to_iso) = result.unwrap();

        // Should fall back to lookback_minutes
        assert!(chrono::DateTime::parse_from_rfc3339(&from_iso).is_ok());
        assert!(chrono::DateTime::parse_from_rfc3339(&to_iso).is_ok());

        // Cleanup
        let _ = tokio::fs::remove_file(path_str).await;
    }

    #[tokio::test]
    async fn test_window_without_watermark_file() {
        let temp_dir = std::env::temp_dir();
        let test_file = temp_dir.join("test_window_nonexistent.txt");
        let path_str = test_file.to_str().unwrap();

        // Ensure file doesn't exist
        let _ = tokio::fs::remove_file(path_str).await;

        let result = window(path_str, 60).await;
        assert!(result.is_ok());

        let (from_iso, to_iso) = result.unwrap();

        // Both should be valid RFC3339 timestamps
        let from = chrono::DateTime::parse_from_rfc3339(&from_iso).unwrap();
        let to = chrono::DateTime::parse_from_rfc3339(&to_iso).unwrap();

        // from should be about 60 minutes before to
        let diff = (to - from).num_minutes();
        assert!((59..=61).contains(&diff)); // Allow 1 minute tolerance
    }

    #[tokio::test]
    async fn test_window_custom_lookback() {
        let temp_dir = std::env::temp_dir();
        let test_file = temp_dir.join("test_window_custom_lookback.txt");
        let path_str = test_file.to_str().unwrap();

        // Ensure file doesn't exist
        let _ = tokio::fs::remove_file(path_str).await;

        // Test with different lookback periods
        for lookback in [30, 90, 180, 360] {
            let result = window(path_str, lookback).await;
            assert!(result.is_ok());

            let (from_iso, to_iso) = result.unwrap();
            let from = chrono::DateTime::parse_from_rfc3339(&from_iso).unwrap();
            let to = chrono::DateTime::parse_from_rfc3339(&to_iso).unwrap();

            let diff = (to - from).num_minutes();
            assert!((lookback - 1..=lookback + 1).contains(&diff));
        }
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
            let payload = payload_from(&doc);

            assert_eq!(payload["kind"], serde_json::json!(kind));
        }
    }
}
