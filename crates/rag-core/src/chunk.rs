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

/// Logical ID of chunk `index` of document `doc_id` (for example `monitor_123#c0`).
pub fn chunk_id(doc_id: &str, index: usize) -> String {
    format!("{doc_id}#c{index}")
}

/// Version of the [`content_hash`] input layout. Changing it invalidates every stored hash
/// and so re-embeds everything once.
const CONTENT_HASH_VERSION: &str = "tails-content-v1";

/// Stable hash (64 hex chars) of everything that determines a document's stored points:
/// every field of `doc` (text, title, URI, kind, timestamp, service, environment and
/// metadata), the chunking parameters and the embedding model. Object keys are sorted
/// before hashing, so metadata key order does not matter.
pub fn content_hash(
    doc: &RagDocument,
    max_chars: usize,
    overlap: usize,
    embedding_model: &str,
) -> String {
    let input = serde_json::json!({
        "version": CONTENT_HASH_VERSION,
        "chunk": {"max_chars": max_chars, "overlap": overlap},
        "embedding_model": embedding_model,
        "doc": doc,
    });
    let bytes = serde_json::to_vec(&canonical(input)).expect("JSON values serialize");
    hex::encode(Sha256::digest(bytes))
}

/// `value` with object keys sorted recursively, independent of serde_json's map ordering.
fn canonical(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut entries: Vec<_> = map.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            serde_json::Value::Object(
                entries
                    .into_iter()
                    .map(|(k, v)| (k, canonical(v)))
                    .collect(),
            )
        }
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(canonical).collect())
        }
        other => other,
    }
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
            id: chunk_id(&doc.id, i),
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
    fn test_content_hash_is_stable_and_ignores_key_order() {
        let mut doc = create_test_doc("Some text");
        doc.metadata.insert("a".into(), serde_json::json!(1));
        doc.metadata
            .insert("b".into(), serde_json::json!({"y": 1, "x": 2}));
        let mut reordered = doc.clone();
        reordered.metadata = serde_json::Map::new();
        reordered
            .metadata
            .insert("b".into(), serde_json::json!({"x": 2, "y": 1}));
        reordered.metadata.insert("a".into(), serde_json::json!(1));

        let hash = content_hash(&doc, 100, 20, "model");
        assert_eq!(hash.len(), 64);
        assert_eq!(hash, content_hash(&doc, 100, 20, "model"));
        assert_eq!(hash, content_hash(&reordered, 100, 20, "model"));
    }

    #[test]
    fn test_content_hash_covers_every_input() {
        let doc = create_test_doc("Some text");
        let base = content_hash(&doc, 100, 20, "model");
        let edited = |edit: fn(&mut RagDocument)| {
            let mut d = doc.clone();
            edit(&mut d);
            content_hash(&d, 100, 20, "model")
        };
        let changed = [
            ("text", edited(|d| d.text = "Other text".into())),
            ("title", edited(|d| d.title = "Other".into())),
            (
                "uri",
                edited(|d| d.source_uri = "http://example.com/x".into()),
            ),
            ("kind", edited(|d| d.kind = SourceKind::Monitor)),
            ("timestamp", edited(|d| d.timestamp = None)),
            ("service", edited(|d| d.service = "other".into())),
            ("environment", edited(|d| d.environment = "other".into())),
            (
                "metadata",
                edited(|d| {
                    d.metadata.insert("k".into(), serde_json::json!("v"));
                }),
            ),
            ("chunk size", content_hash(&doc, 101, 20, "model")),
            ("overlap", content_hash(&doc, 100, 21, "model")),
            ("model", content_hash(&doc, 100, 20, "other-model")),
        ];
        for (what, hash) in changed {
            assert_ne!(hash, base, "{what} must change the hash");
        }
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
