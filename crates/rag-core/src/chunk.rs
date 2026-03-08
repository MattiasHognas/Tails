// crates/rag-core/src/chunk.rs
use crate::domain::RagDocument;
use sha2::{Digest, Sha256};

/// Stable 16-byte (32 hex chars) ID derived from parts
pub fn stable_id(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(parts.join("|"));
    let out = hasher.finalize();
    hex::encode(&out[..16])
}

/// Split a document into overlapping character-length chunks.
/// - `max_chars`: maximum characters per chunk (not bytes)
/// - `overlap`: characters of overlap between consecutive chunks
pub fn chunk(max_chars: usize, overlap: usize, doc: &RagDocument) -> Vec<RagDocument> {
    // Edge cases
    if doc.text.is_empty() || max_chars == 0 {
        return vec![];
    }
    // Prevent infinite loop when overlap >= max_chars
    let step = if overlap >= max_chars {
        max_chars
    } else {
        max_chars - overlap
    };

    let chars: Vec<char> = doc.text.chars().collect();
    let n = chars.len();

    let mut out = Vec::new();
    let mut i = 0usize;
    let mut start = 0usize;

    while start < n {
        let end = usize::min(n, start + max_chars);
        let piece: String = chars[start..end].iter().collect();

        // Inherit and enrich metadata
        let mut md = doc.metadata.clone();
        md.insert("chunk_index".into(), i.into());
        md.insert("chunk_of".into(), doc.id.clone().into());

        out.push(RagDocument {
            id: format!("{}#c{}", doc.id, i),
            title: doc.title.clone(),
            text: piece,
            source_uri: doc.source_uri.clone(),
            kind: doc.kind.clone(),
            timestamp: doc.timestamp.clone(),
            service: doc.service.clone(),
            environment: doc.environment.clone(),
            metadata: md,
        });

        if end == n {
            break;
        }
        i += 1;
        // Advance by step (max_chars - overlap)
        start = start.saturating_add(step);
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::SourceKind;

    fn create_test_doc(text: &str) -> RagDocument {
        RagDocument {
            id: "test_id".to_string(),
            title: "Test Title".to_string(),
            text: text.to_string(),
            source_uri: "http://example.com".to_string(),
            kind: SourceKind::Logs,
            timestamp: Some("2025-01-01T00:00:00Z".to_string()),
            service: "test-service".to_string(),
            environment: "test".to_string(),
            metadata: serde_json::Map::new(),
        }
    }

    #[test]
    fn test_stable_id_consistency() {
        let id1 = stable_id(&["part1", "part2", "part3"]);
        let id2 = stable_id(&["part1", "part2", "part3"]);
        assert_eq!(id1, id2);
        assert_eq!(id1.len(), 32); // 16 bytes * 2 hex chars
    }

    #[test]
    fn test_stable_id_different_inputs() {
        let id1 = stable_id(&["part1", "part2"]);
        let id2 = stable_id(&["part1", "part3"]);
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_stable_id_order_matters() {
        let id1 = stable_id(&["part1", "part2"]);
        let id2 = stable_id(&["part2", "part1"]);
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_chunk_empty_text() {
        let doc = create_test_doc("");
        let chunks = chunk(100, 20, &doc);
        assert_eq!(chunks.len(), 0);
    }

    #[test]
    fn test_chunk_zero_max_chars() {
        let doc = create_test_doc("Some text");
        let chunks = chunk(0, 20, &doc);
        assert_eq!(chunks.len(), 0);
    }

    #[test]
    fn test_chunk_short_text_single_chunk() {
        let doc = create_test_doc("Short text");
        let chunks = chunk(100, 20, &doc);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, "Short text");
        assert_eq!(chunks[0].id, "test_id#c0");
        assert_eq!(
            chunks[0]
                .metadata
                .get("chunk_index")
                .unwrap()
                .as_u64()
                .unwrap(),
            0
        );
        assert_eq!(
            chunks[0]
                .metadata
                .get("chunk_of")
                .unwrap()
                .as_str()
                .unwrap(),
            "test_id"
        );
    }

    #[test]
    fn test_chunk_multiple_chunks_no_overlap() {
        let doc = create_test_doc("0123456789");
        let chunks = chunk(5, 0, &doc);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].text, "01234");
        assert_eq!(chunks[1].text, "56789");
        assert_eq!(chunks[0].id, "test_id#c0");
        assert_eq!(chunks[1].id, "test_id#c1");
    }

    #[test]
    fn test_chunk_with_overlap() {
        let doc = create_test_doc("0123456789");
        let chunks = chunk(5, 2, &doc);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].text, "01234");
        assert_eq!(chunks[1].text, "34567");
        assert_eq!(chunks[2].text, "6789");
    }

    #[test]
    fn test_chunk_overlap_equal_to_max_chars() {
        let doc = create_test_doc("0123456789");
        let chunks = chunk(5, 5, &doc);
        // When overlap >= max_chars, step = max_chars, so we advance by max_chars each time
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].text, "01234");
        assert_eq!(chunks[1].text, "56789");
    }

    #[test]
    fn test_chunk_overlap_greater_than_max_chars() {
        let doc = create_test_doc("0123456789");
        let chunks = chunk(5, 10, &doc);
        // When overlap > max_chars, step = max_chars
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].text, "01234");
        assert_eq!(chunks[1].text, "56789");
    }

    #[test]
    fn test_chunk_unicode_characters() {
        let doc = create_test_doc("Hello 世界 🌍");
        let chunks = chunk(8, 2, &doc);
        assert!(!chunks.is_empty());
        // Verify that chunks are based on character count, not byte count
        assert!(chunks[0].text.chars().count() <= 8);
    }

    #[test]
    fn test_chunk_preserves_document_metadata() {
        let doc = create_test_doc("Test document");
        let chunks = chunk(100, 20, &doc);
        assert_eq!(chunks[0].title, "Test Title");
        assert_eq!(chunks[0].service, "test-service");
        assert_eq!(chunks[0].environment, "test");
        assert_eq!(chunks[0].source_uri, "http://example.com");
        assert_eq!(chunks[0].kind, SourceKind::Logs);
        assert_eq!(
            chunks[0].timestamp,
            Some("2025-01-01T00:00:00Z".to_string())
        );
    }

    #[test]
    fn test_chunk_inherits_and_enriches_metadata() {
        let mut doc = create_test_doc("Test text");
        doc.metadata
            .insert("custom_key".to_string(), serde_json::json!("custom_value"));

        let chunks = chunk(100, 20, &doc);
        assert_eq!(
            chunks[0]
                .metadata
                .get("custom_key")
                .unwrap()
                .as_str()
                .unwrap(),
            "custom_value"
        );
        assert!(chunks[0].metadata.contains_key("chunk_index"));
        assert!(chunks[0].metadata.contains_key("chunk_of"));
    }
}
