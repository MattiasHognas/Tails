// crates/rag-core/src/chunk.rs
use crate::domain::{RagDocument, SourceKind};
use crate::text::truncate_bytes;
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

/// Version of the [`content_hash`] input layout and of what is embedded for a chunk
/// ([`embedding_input`]). Changing either invalidates every stored hash and so re-embeds
/// everything once.
///
/// - v1: the chunk text was embedded.
/// - v2: [`embedding_input`], a context header plus the chunk text, is embedded.
const CONTENT_HASH_VERSION: &str = "tails-content-v2";

/// Longest title in the embedding header, in bytes (cut at a char boundary).
const HEADER_TITLE_MAX_BYTES: usize = 300;
/// Longest field value in the embedding header, in bytes (cut at a char boundary).
const HEADER_VALUE_MAX_BYTES: usize = 100;
/// Upper bound of the embedding header's length in chars: the kind label and title, then
/// at most four fields (service, environment and two metadata values) with their labels.
pub const EMBEDDING_HEADER_MAX_CHARS: usize =
    16 + HEADER_TITLE_MAX_BYTES + 4 * (16 + HEADER_VALUE_MAX_BYTES);

/// Label of a kind in the embedding header.
fn kind_label(kind: &SourceKind) -> &'static str {
    match kind {
        SourceKind::Logs => "Log",
        SourceKind::Metrics => "Metric",
        SourceKind::Monitor => "Monitor",
        SourceKind::Incident => "Incident",
        SourceKind::Dashboard => "Dashboard",
        SourceKind::SLO => "SLO",
        SourceKind::Git => "Git",
    }
}

/// High-signal metadata per kind as (header label, metadata key).
fn header_fields(kind: &SourceKind) -> &'static [(&'static str, &'static str)] {
    match kind {
        SourceKind::Incident => &[("severity", "severity"), ("state", "state")],
        SourceKind::Logs => &[("status", "status")],
        SourceKind::Monitor => &[("type", "monitor_type")],
        SourceKind::SLO => &[("type", "slo_type"), ("target", "target")],
        SourceKind::Metrics | SourceKind::Dashboard | SourceKind::Git => &[],
    }
}

/// `value` on one line (whitespace collapsed), cut to `max_bytes`.
fn header_value(value: &str, max_bytes: usize) -> String {
    let flat = value.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_bytes(&flat, max_bytes).to_string()
}

/// What is embedded for a chunk: a short context header, a blank line, then the chunk
/// text. The stored `Text` and the answer prompt are unaffected.
///
/// ```text
/// [Incident] Checkout latency above 2s
/// service: checkout · env: prod · severity: SEV-2 · state: resolved
///
/// <chunk text>
/// ```
///
/// Every chunk of a document gets the header, so later chunks of a long document still
/// carry its title, and a log's vector carries its service even when the message never
/// names it. Empty fields are left out. Everything in it is a field of `doc`, so
/// [`content_hash`] covers it; it adds at most [`EMBEDDING_HEADER_MAX_CHARS`] chars.
pub fn embedding_input(doc: &RagDocument) -> String {
    let title = header_value(&doc.title, HEADER_TITLE_MAX_BYTES);
    let mut out = format!("[{}]", kind_label(&doc.kind));
    if !title.is_empty() {
        out.push(' ');
        out.push_str(&title);
    }
    let mut fields: Vec<String> = vec![];
    for (label, value) in [("service", &doc.service), ("env", &doc.environment)] {
        let value = header_value(value, HEADER_VALUE_MAX_BYTES);
        if !value.is_empty() {
            fields.push(format!("{label}: {value}"));
        }
    }
    for (label, key) in header_fields(&doc.kind) {
        let value = match doc.metadata.get(*key) {
            Some(serde_json::Value::String(s)) => header_value(s, HEADER_VALUE_MAX_BYTES),
            Some(serde_json::Value::Number(n)) => n.to_string(),
            _ => String::new(),
        };
        if !value.is_empty() {
            let unit = if *key == "target" { "%" } else { "" };
            fields.push(format!("{label}: {value}{unit}"));
        }
    }
    if !fields.is_empty() {
        out.push('\n');
        out.push_str(&fields.join(" · "));
    }
    out.push_str("\n\n");
    out.push_str(&doc.text);
    out
}

/// Stable hash (64 hex chars) of everything that determines a document's stored points:
/// every field of `doc` (text, title, URI, kind, timestamp, service, environment and
/// metadata, which includes everything [`embedding_input`] adds), the chunking
/// parameters and the embedding model. Object keys are sorted before hashing, so
/// metadata key order does not matter.
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
    fn test_embedding_input_header_per_kind() {
        let mut incident = create_test_doc("Checkout p95 above 2s for 40 minutes.");
        incident.kind = SourceKind::Incident;
        incident.title = "Checkout latency".into();
        incident.service = "checkout".into();
        incident.environment = "prod".into();
        incident
            .metadata
            .insert("severity".into(), serde_json::json!("SEV-2"));
        incident
            .metadata
            .insert("state".into(), serde_json::json!("resolved"));
        incident
            .metadata
            .insert("customer_impact".into(), serde_json::json!("EU"));
        assert_eq!(
            embedding_input(&incident),
            "[Incident] Checkout latency\n\
             service: checkout · env: prod · severity: SEV-2 · state: resolved\n\n\
             Checkout p95 above 2s for 40 minutes."
        );

        let mut slo = create_test_doc("text");
        slo.kind = SourceKind::SLO;
        slo.title = "Checkout availability".into();
        slo.metadata
            .insert("slo_type".into(), serde_json::json!("metric"));
        slo.metadata
            .insert("target".into(), serde_json::json!(99.9));
        assert!(
            embedding_input(&slo).starts_with(
                "[SLO] Checkout availability\n\
                 service: test-service · env: test · type: metric · target: 99.9%\n\n"
            ),
            "{}",
            embedding_input(&slo)
        );

        let mut monitor = create_test_doc("text");
        monitor.kind = SourceKind::Monitor;
        monitor
            .metadata
            .insert("monitor_type".into(), serde_json::json!("query alert"));
        assert!(embedding_input(&monitor).contains("· type: query alert\n\n"));

        let mut log = create_test_doc("lock wait timeout exceeded");
        log.title = "Log: inventory - error".into();
        log.service = "inventory".into();
        log.metadata
            .insert("status".into(), serde_json::json!("error"));
        assert_eq!(
            embedding_input(&log),
            "[Log] Log: inventory - error\nservice: inventory · env: test · status: error\n\n\
             lock wait timeout exceeded"
        );
    }

    #[test]
    fn test_embedding_input_omits_empty_fields() {
        let mut dashboard = create_test_doc("Latency overview");
        dashboard.kind = SourceKind::Dashboard;
        dashboard.title = "Payments".into();
        dashboard.service = String::new();
        dashboard.environment = "  ".into();
        dashboard
            .metadata
            .insert("author".into(), serde_json::json!("a@b.c"));
        assert_eq!(
            embedding_input(&dashboard),
            "[Dashboard] Payments\n\nLatency overview"
        );

        let mut bare = create_test_doc("x");
        bare.kind = SourceKind::Incident;
        bare.title = String::new();
        bare.service = String::new();
        bare.environment = String::new();
        bare.metadata
            .insert("severity".into(), serde_json::json!(""));
        bare.metadata
            .insert("state".into(), serde_json::Value::Null);
        assert_eq!(embedding_input(&bare), "[Incident]\n\nx");
    }

    /// Every chunk carries the document's header, so later chunks keep the title.
    #[test]
    fn test_embedding_input_is_added_to_every_chunk() {
        let mut doc = create_test_doc(&"runbook step. ".repeat(40));
        doc.title = "Ledger reconciliation drift".into();
        let chunks = chunk(100, 10, &doc);
        assert!(chunks.len() > 3);
        for c in &chunks {
            let input = embedding_input(c);
            assert!(
                input.starts_with("[Log] Ledger reconciliation drift\n"),
                "{input}"
            );
            assert!(input.ends_with(&c.text));
            // The stored text itself is unchanged.
            assert!(!c.text.contains("Ledger"));
        }
    }

    #[test]
    fn test_embedding_input_header_is_bounded_and_utf8_safe() {
        let mut doc = create_test_doc("body");
        doc.kind = SourceKind::Incident;
        doc.title = format!("{}\n\t{}", "å".repeat(400), "決済".repeat(100));
        doc.service = "🚀".repeat(200);
        doc.environment = "e\u{0301}".repeat(200);
        doc.metadata
            .insert("severity".into(), serde_json::json!("ö".repeat(300)));
        doc.metadata
            .insert("state".into(), serde_json::json!("👩\u{200D}💻".repeat(50)));
        let input = embedding_input(&doc);
        let header = input.strip_suffix("\n\nbody").unwrap();
        assert!(!header.contains('\t'));
        assert_eq!(header.lines().count(), 2);
        assert!(
            header.chars().count() <= EMBEDDING_HEADER_MAX_CHARS,
            "{} chars",
            header.chars().count()
        );
        // Cuts never leave a dangling combining mark or joiner.
        assert!(!header.contains(" \u{0301}") && !header.ends_with('\u{200D}'));
        assert!(header.contains(&"å".repeat(HEADER_TITLE_MAX_BYTES / 2)));
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
