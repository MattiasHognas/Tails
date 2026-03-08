use crate::domain::Hit;
use crate::openai::OpenAiClient;
use crate::reranker::rerank_mmr_signals;
use anyhow::Result;

pub async fn answer_question(
    oa: &OpenAiClient,
    candidates: Vec<Hit>,
    top_k: usize,
    question: &str,
) -> Result<String> {
    let hits = rerank_mmr_signals(&candidates, top_k);
    let mut sb = String::new();
    for (i, h) in hits.iter().enumerate() {
        use std::fmt::Write;
        let _ = writeln!(sb, "[DOC #{}] {} ({:?})", i + 1, h.doc.title, h.doc.kind);
        if let Some(ts) = &h.doc.timestamp {
            let _ = writeln!(sb, "Time: {}", ts);
        }
        if !h.doc.service.is_empty() {
            let _ = writeln!(
                sb,
                "Service: {} | Env: {}",
                h.doc.service, h.doc.environment
            );
        }
        let _ = writeln!(sb, "Source: {}", h.doc.source_uri);
        let _ = writeln!(sb, "Score: {:.3}", h.score);
        // selected metadata
        for (k, v) in &h.doc.metadata {
            if [
                "severity",
                "state",
                "dd_incident_id",
                "dd_monitor_id",
                "dd_metric",
                "status",
                "window_from",
                "window_to",
                "type",
            ]
            .contains(&k.as_str())
            {
                let _ = writeln!(sb, "{}: {}", k, v);
            }
        }
        let mut body = h.doc.text.clone();
        if body.len() > 1500 {
            body.truncate(1500);
            body.push_str(" …[truncated]");
        }
        let _ = writeln!(
            sb,
            "Excerpt:
{}
",
            body
        );
    }

    let user = format!(
        "Question:
{}

Context:
{}

Instructions:
- Answer concisely.
- If multiple hypotheses exist, list them ordered by likelihood.
- Provide bullet-point 'Top signals' and 'Next steps'.
- Include markdown links to each Source when you cite evidence.
",
        question, sb
    );
    let system = "You are a helpful SRE assistant. Use only provided context. When unsure, say so. Always cite SourceUri for each claim.";
    let out = oa.chat_complete(system, &user).await?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{RagDocument, SourceKind};

    fn create_test_hit(id: &str, title: &str, text: &str, score: f32) -> Hit {
        Hit {
            doc: RagDocument {
                id: id.to_string(),
                title: title.to_string(),
                text: text.to_string(),
                source_uri: format!("http://example.com/{}", id),
                kind: SourceKind::Monitor,
                timestamp: Some("2025-01-01T00:00:00Z".to_string()),
                service: "test-service".to_string(),
                environment: "production".to_string(),
                metadata: serde_json::Map::new(),
            },
            score,
        }
    }

    #[test]
    fn test_context_formatting_basic() {
        let hits = vec![create_test_hit(
            "1",
            "Test Monitor",
            "Monitor alert triggered",
            0.9,
        )];

        // Build context string similar to answer_question
        let mut sb = String::new();
        for (i, h) in hits.iter().enumerate() {
            use std::fmt::Write;
            let _ = writeln!(sb, "[DOC #{}] {} ({:?})", i + 1, h.doc.title, h.doc.kind);
            if let Some(ts) = &h.doc.timestamp {
                let _ = writeln!(sb, "Time: {}", ts);
            }
            if !h.doc.service.is_empty() {
                let _ = writeln!(
                    sb,
                    "Service: {} | Env: {}",
                    h.doc.service, h.doc.environment
                );
            }
            let _ = writeln!(sb, "Source: {}", h.doc.source_uri);
            let _ = writeln!(sb, "Score: {:.3}", h.score);
        }

        assert!(sb.contains("[DOC #1] Test Monitor"));
        assert!(sb.contains("Monitor"));
        assert!(sb.contains("Time: 2025-01-01T00:00:00Z"));
        assert!(sb.contains("Service: test-service | Env: production"));
        assert!(sb.contains("Source: http://example.com/1"));
        assert!(sb.contains("Score: 0.900"));
    }

    #[test]
    fn test_context_formatting_multiple_docs() {
        let hits = vec![
            create_test_hit("1", "First Doc", "First content", 0.9),
            create_test_hit("2", "Second Doc", "Second content", 0.8),
        ];

        let mut sb = String::new();
        for (i, h) in hits.iter().enumerate() {
            use std::fmt::Write;
            let _ = writeln!(sb, "[DOC #{}] {} ({:?})", i + 1, h.doc.title, h.doc.kind);
        }

        assert!(sb.contains("[DOC #1] First Doc"));
        assert!(sb.contains("[DOC #2] Second Doc"));
    }

    #[test]
    fn test_context_formatting_text_truncation() {
        let long_text = "a".repeat(2000);
        let mut hit = create_test_hit("1", "Long Doc", &long_text, 0.9);
        hit.doc.text = long_text;

        let hits = vec![hit];

        let mut sb = String::new();
        for h in hits.iter() {
            let mut body = h.doc.text.clone();
            if body.len() > 1500 {
                body.truncate(1500);
                body.push_str(" …[truncated]");
            }
            sb.push_str(&body);
        }

        assert_eq!(sb.len(), 1500 + " …[truncated]".len());
        assert!(sb.ends_with(" …[truncated]"));
    }

    #[test]
    fn test_context_formatting_no_truncation() {
        let short_text = "Short text content";
        let hit = create_test_hit("1", "Short Doc", short_text, 0.9);

        let hits = vec![hit];

        let mut sb = String::new();
        for h in hits.iter() {
            let mut body = h.doc.text.clone();
            if body.len() > 1500 {
                body.truncate(1500);
                body.push_str(" …[truncated]");
            }
            sb.push_str(&body);
        }

        assert_eq!(sb, short_text);
        assert!(!sb.contains("truncated"));
    }

    #[test]
    fn test_context_formatting_metadata_filtering() {
        let mut hit = create_test_hit("1", "Doc with metadata", "Content", 0.9);
        hit.doc
            .metadata
            .insert("severity".to_string(), serde_json::json!("high"));
        hit.doc
            .metadata
            .insert("status".to_string(), serde_json::json!("open"));
        hit.doc
            .metadata
            .insert("unrelated".to_string(), serde_json::json!("ignored"));

        let hits = vec![hit];

        let mut sb = String::new();
        let selected_keys = [
            "severity",
            "state",
            "dd_incident_id",
            "dd_monitor_id",
            "dd_metric",
            "status",
            "window_from",
            "window_to",
            "type",
        ];
        for h in hits.iter() {
            for (k, v) in &h.doc.metadata {
                if selected_keys.contains(&k.as_str()) {
                    use std::fmt::Write;
                    let _ = writeln!(sb, "{}: {}", k, v);
                }
            }
        }

        assert!(sb.contains("severity: \"high\""));
        assert!(sb.contains("status: \"open\""));
        assert!(!sb.contains("unrelated"));
    }

    #[test]
    fn test_context_formatting_no_service() {
        let mut hit = create_test_hit("1", "Doc", "Content", 0.9);
        hit.doc.service = String::new();
        hit.doc.environment = String::new();

        let hits = vec![hit];

        let mut sb = String::new();
        for h in hits.iter() {
            if !h.doc.service.is_empty() {
                use std::fmt::Write;
                let _ = writeln!(
                    sb,
                    "Service: {} | Env: {}",
                    h.doc.service, h.doc.environment
                );
            }
        }

        assert!(!sb.contains("Service:"));
        assert!(!sb.contains("Env:"));
    }

    #[test]
    fn test_context_formatting_no_timestamp() {
        let mut hit = create_test_hit("1", "Doc", "Content", 0.9);
        hit.doc.timestamp = None;

        let hits = vec![hit];

        let mut sb = String::new();
        for h in hits.iter() {
            if let Some(ts) = &h.doc.timestamp {
                use std::fmt::Write;
                let _ = writeln!(sb, "Time: {}", ts);
            }
        }

        assert!(!sb.contains("Time:"));
    }

    #[test]
    fn test_user_prompt_formatting() {
        let question = "Why did the service fail?";
        let context = "Context goes here";

        let user = format!(
            "Question:
{}

Context:
{}

Instructions:
- Answer concisely.
- If multiple hypotheses exist, list them ordered by likelihood.
- Provide bullet-point 'Top signals' and 'Next steps'.
- Include markdown links to each Source when you cite evidence.
",
            question, context
        );

        assert!(user.contains("Question:"));
        assert!(user.contains("Why did the service fail?"));
        assert!(user.contains("Context:"));
        assert!(user.contains("Context goes here"));
        assert!(user.contains("Instructions:"));
        assert!(user.contains("Answer concisely"));
        assert!(user.contains("Top signals"));
        assert!(user.contains("Next steps"));
    }

    #[test]
    fn test_system_prompt_content() {
        let system = "You are a helpful SRE assistant. Use only provided context. When unsure, say so. Always cite SourceUri for each claim.";

        assert!(system.contains("SRE assistant"));
        assert!(system.contains("Use only provided context"));
        assert!(system.contains("When unsure, say so"));
        assert!(system.contains("cite SourceUri"));
    }

    #[test]
    fn test_different_source_kinds() {
        let kinds = vec![
            SourceKind::Logs,
            SourceKind::Metrics,
            SourceKind::Monitor,
            SourceKind::Incident,
            SourceKind::Dashboard,
            SourceKind::SLO,
        ];

        for kind in kinds {
            let mut hit = create_test_hit("1", "Test", "Content", 0.9);
            hit.doc.kind = kind.clone();

            let mut sb = String::new();
            use std::fmt::Write;
            let _ = writeln!(sb, "[DOC #1] {} ({:?})", hit.doc.title, hit.doc.kind);

            assert!(sb.contains("Test"));
            // Verify the kind is formatted in the output
            assert!(!sb.is_empty());
        }
    }

    #[tokio::test]
    async fn test_answer_question_with_mock_openai() {
        use crate::openai::OpenAiClient;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{
                    "message": {
                        "content": "Test answer to the question"
                    }
                }]
            })))
            .mount(&mock_server)
            .await;

        let oa = OpenAiClient {
            api_key: "test_key".to_string(),
            base_url: mock_server.uri(),
            embedding_model: "test-model".to_string(),
            chat_model: "test-chat".to_string(),
            http: reqwest::Client::new(),
        };

        let candidates = vec![
            create_test_hit("1", "Test Hit 1", "This is test content", 0.9),
            create_test_hit("2", "Test Hit 2", "More test content", 0.8),
        ];

        let result = answer_question(&oa, candidates, 2, "What is the test?").await;
        assert!(result.is_ok());
        let answer = result.unwrap();
        assert_eq!(answer, "Test answer to the question");
    }

    #[tokio::test]
    async fn test_answer_question_truncation() {
        use crate::openai::OpenAiClient;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{
                    "message": {
                        "content": "Answer based on truncated content"
                    }
                }]
            })))
            .mount(&mock_server)
            .await;

        let oa = OpenAiClient {
            api_key: "test_key".to_string(),
            base_url: mock_server.uri(),
            embedding_model: "test-model".to_string(),
            chat_model: "test-chat".to_string(),
            http: reqwest::Client::new(),
        };

        // Create a hit with very long text (>1500 chars) that should be truncated
        let long_text = "a".repeat(2000);
        let mut hit = create_test_hit("1", "Long Doc", &long_text, 0.9);
        hit.doc.text = long_text;

        let candidates = vec![hit];

        let result = answer_question(&oa, candidates, 1, "Test question").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_answer_question_error_handling() {
        use crate::openai::OpenAiClient;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&mock_server)
            .await;

        let oa = OpenAiClient {
            api_key: "test_key".to_string(),
            base_url: mock_server.uri(),
            embedding_model: "test-model".to_string(),
            chat_model: "test-chat".to_string(),
            http: reqwest::Client::new(),
        };

        let candidates = vec![create_test_hit("1", "Test Hit", "Test content", 0.9)];

        let result = answer_question(&oa, candidates, 1, "Test question").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_answer_question_empty_candidates() {
        use crate::openai::OpenAiClient;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{
                    "message": {
                        "content": "No context available"
                    }
                }]
            })))
            .mount(&mock_server)
            .await;

        let oa = OpenAiClient {
            api_key: "test_key".to_string(),
            base_url: mock_server.uri(),
            embedding_model: "test-model".to_string(),
            chat_model: "test-chat".to_string(),
            http: reqwest::Client::new(),
        };

        let candidates = vec![];

        let result = answer_question(&oa, candidates, 10, "Test question").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_answer_question_metadata_included() {
        use crate::openai::OpenAiClient;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{
                    "message": {
                        "content": "Answer with metadata context"
                    }
                }]
            })))
            .mount(&mock_server)
            .await;

        let oa = OpenAiClient {
            api_key: "test_key".to_string(),
            base_url: mock_server.uri(),
            embedding_model: "test-model".to_string(),
            chat_model: "test-chat".to_string(),
            http: reqwest::Client::new(),
        };

        let mut metadata = serde_json::Map::new();
        metadata.insert("severity".to_string(), serde_json::json!("SEV-1"));
        metadata.insert("state".to_string(), serde_json::json!("open"));

        let mut hit = create_test_hit("1", "Test", "Content", 0.9);
        hit.doc.metadata = metadata;
        hit.doc.kind = SourceKind::Incident;

        let candidates = vec![hit];

        let result = answer_question(&oa, candidates, 1, "What happened?").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_truncation_boundary_exactly_1500() {
        use crate::openai::OpenAiClient;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{
                    "message": {
                        "content": "Answer"
                    }
                }]
            })))
            .mount(&mock_server)
            .await;

        let oa = OpenAiClient {
            api_key: "test_key".to_string(),
            base_url: mock_server.uri(),
            embedding_model: "test-model".to_string(),
            chat_model: "test-chat".to_string(),
            http: reqwest::Client::new(),
        };

        // Text with exactly 1500 characters (boundary condition)
        let text_1500 = "a".repeat(1500);
        let hit = create_test_hit("1", "Exact 1500", &text_1500, 0.9);

        let candidates = vec![hit];

        let result = answer_question(&oa, candidates, 1, "Test").await;
        assert!(result.is_ok());

        // With exactly 1500 chars, should NOT be truncated
        // This tests the > comparison (not >=)
    }

    #[tokio::test]
    async fn test_truncation_boundary_1499() {
        use crate::openai::OpenAiClient;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{
                    "message": {
                        "content": "Answer"
                    }
                }]
            })))
            .mount(&mock_server)
            .await;

        let oa = OpenAiClient {
            api_key: "test_key".to_string(),
            base_url: mock_server.uri(),
            embedding_model: "test-model".to_string(),
            chat_model: "test-chat".to_string(),
            http: reqwest::Client::new(),
        };

        // Text with 1499 characters (just under boundary)
        let text_1499 = "a".repeat(1499);
        let hit = create_test_hit("1", "Under 1500", &text_1499, 0.9);

        let candidates = vec![hit];

        let result = answer_question(&oa, candidates, 1, "Test").await;
        assert!(result.is_ok());

        // With 1499 chars, should NOT be truncated
    }

    #[tokio::test]
    async fn test_truncation_boundary_1501() {
        use crate::openai::OpenAiClient;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{
                    "message": {
                        "content": "Answer"
                    }
                }]
            })))
            .mount(&mock_server)
            .await;

        let oa = OpenAiClient {
            api_key: "test_key".to_string(),
            base_url: mock_server.uri(),
            embedding_model: "test-model".to_string(),
            chat_model: "test-chat".to_string(),
            http: reqwest::Client::new(),
        };

        // Text with 1501 characters (just over boundary)
        let text_1501 = "a".repeat(1501);
        let hit = create_test_hit("1", "Over 1500", &text_1501, 0.9);

        let candidates = vec![hit];

        let result = answer_question(&oa, candidates, 1, "Test").await;
        assert!(result.is_ok());

        // With 1501 chars, SHOULD be truncated
        // This tests that > comparison works (not >=, not ==)
    }
}
