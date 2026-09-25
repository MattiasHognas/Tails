use crate::error::{RagError, Stage, UpstreamError};
use crate::resilience::{HttpConfig, RetryPolicy, send_with_retry};
use anyhow::Result;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

const EMBEDDINGS_PATH: &str = "/v1/embeddings";

/// An OpenAI-compatible API: chat completions at `base_url`, embeddings at
/// `embedding_base_url` (the same server unless configured otherwise, e.g. a
/// self-hosted embedding server such as text-embeddings-inference).
#[derive(Debug, Clone)]
pub struct OpenAiClient {
    pub api_key: String,
    pub base_url: String,
    /// Where `/v1/embeddings` is sent (`OPENAI_EMBEDDING_BASE_URL`).
    pub embedding_base_url: String,
    /// Bearer key for the embeddings endpoint (`OPENAI_EMBEDDING_API_KEY`).
    pub embedding_api_key: String,
    pub embedding_model: String,
    pub chat_model: String,
    pub http: reqwest::Client,
    pub retry: RetryPolicy,
}

#[derive(Serialize)]
struct Msg<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct ChatResp {
    choices: Vec<Choice>,
}
#[derive(Deserialize)]
struct Choice {
    message: Message,
}
#[derive(Deserialize)]
struct Message {
    content: Option<String>,
}

impl OpenAiClient {
    pub fn new(
        api_key: String,
        base_url: String,
        embedding_model: String,
        chat_model: String,
    ) -> Self {
        Self {
            embedding_api_key: api_key.clone(),
            embedding_base_url: base_url.clone(),
            api_key,
            base_url,
            embedding_model,
            chat_model,
            http: HttpConfig::default().build_client(),
            retry: RetryPolicy::default(),
        }
    }

    /// `OPENAI_API_KEY` (required), `OPENAI_BASE_URL`, `OPENAI_EMBEDDING_MODEL`,
    /// `OPENAI_CHAT_MODEL`, and for embeddings served elsewhere
    /// `OPENAI_EMBEDDING_BASE_URL` and `OPENAI_EMBEDDING_API_KEY`, which fall back to
    /// `OPENAI_BASE_URL` and `OPENAI_API_KEY` when unset or blank.
    pub fn new_from_env() -> Result<Self> {
        let set = |name: &str| {
            std::env::var(name)
                .ok()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let mut client = Self::new(
            std::env::var("OPENAI_API_KEY")?,
            std::env::var("OPENAI_BASE_URL").unwrap_or_else(|_| "https://api.openai.com".into()),
            std::env::var("OPENAI_EMBEDDING_MODEL")
                .unwrap_or_else(|_| "text-embedding-3-small".into()),
            std::env::var("OPENAI_CHAT_MODEL").unwrap_or_else(|_| "o4-mini".into()),
        );
        if let Some(url) = set("OPENAI_EMBEDDING_BASE_URL") {
            client.embedding_base_url = url;
        }
        if let Some(key) = set("OPENAI_EMBEDDING_API_KEY") {
            client.embedding_api_key = key;
        }
        client.http = HttpConfig::from_env().build_client();
        client.retry = RetryPolicy::from_env();
        Ok(client)
    }

    /// POST `body` to `path` with retries and decode the JSON response.
    /// Failures are attributed to `stage`.
    async fn post_json<B: Serialize, T: DeserializeOwned>(
        &self,
        stage: Stage,
        path: &str,
        body: &B,
    ) -> Result<T, RagError> {
        let (base, key) = if path == EMBEDDINGS_PATH {
            (&self.embedding_base_url, &self.embedding_api_key)
        } else {
            (&self.base_url, &self.api_key)
        };
        let url = format!("{base}{path}");
        let what = format!("openai {path}");
        let r = send_with_retry(&self.retry, &what, || {
            self.http.post(&url).bearer_auth(key).json(body)
        })
        .await
        .map_err(|f| RagError::upstream(stage, f))?;
        r.json::<T>()
            .await
            .map_err(|e| RagError::failed(stage, UpstreamError::from_reqwest(e)))
    }

    async fn chat(&self, stage: Stage, body: &impl Serialize) -> Result<String, RagError> {
        let v: ChatResp = self.post_json(stage, "/v1/chat/completions", body).await?;
        match v.choices.into_iter().next().and_then(|c| c.message.content) {
            Some(content) if !content.trim().is_empty() => Ok(content),
            _ => Err(RagError::failed(
                stage,
                UpstreamError::InvalidResponse("empty chat completion".into()),
            )),
        }
    }

    /// Embed `text`. Never returns an empty vector: a response without an
    /// embedding is an [`RagError::EmbeddingFailed`].
    pub async fn embed(&self, text: &str) -> Result<Vec<f32>, RagError> {
        #[derive(Serialize)]
        struct Req<'a> {
            input: &'a str,
            model: &'a str,
        }
        #[derive(Deserialize)]
        struct Resp {
            data: Vec<Item>,
        }
        #[derive(Deserialize)]
        struct Item {
            embedding: Vec<f32>,
        }
        let v: Resp = self
            .post_json(
                Stage::Embedding,
                EMBEDDINGS_PATH,
                &Req {
                    input: text,
                    model: &self.embedding_model,
                },
            )
            .await?;
        match v.data.into_iter().next() {
            Some(d) if !d.embedding.is_empty() => Ok(d.embedding),
            _ => Err(RagError::failed(
                Stage::Embedding,
                UpstreamError::InvalidResponse("empty embedding".into()),
            )),
        }
    }

    /// Embed `texts` in one request using the array form of `input`. Embeddings are
    /// returned in input order, mapped back by the response `index` field; a missing,
    /// duplicate, out-of-range or empty embedding is an [`RagError::EmbeddingFailed`].
    /// The caller keeps the request within the provider's limits, for example with
    /// [`embedding_batches`].
    pub async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, RagError> {
        #[derive(Serialize)]
        struct Req<'a> {
            input: &'a [String],
            model: &'a str,
        }
        #[derive(Deserialize)]
        struct Resp {
            data: Vec<Item>,
        }
        #[derive(Deserialize)]
        struct Item {
            index: usize,
            embedding: Vec<f32>,
        }
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let v: Resp = self
            .post_json(
                Stage::Embedding,
                EMBEDDINGS_PATH,
                &Req {
                    input: texts,
                    model: &self.embedding_model,
                },
            )
            .await?;
        let invalid =
            |msg: String| RagError::failed(Stage::Embedding, UpstreamError::InvalidResponse(msg));
        let mut out: Vec<Option<Vec<f32>>> = vec![None; texts.len()];
        for item in v.data {
            let slot = out
                .get_mut(item.index)
                .ok_or_else(|| invalid(format!("embedding index {} out of range", item.index)))?;
            if item.embedding.is_empty() {
                return Err(invalid(format!("empty embedding at index {}", item.index)));
            }
            if slot.replace(item.embedding).is_some() {
                return Err(invalid(format!("duplicate embedding index {}", item.index)));
            }
        }
        out.into_iter()
            .enumerate()
            .map(|(i, e)| e.ok_or_else(|| invalid(format!("missing embedding for input {i}"))))
            .collect()
    }

    pub async fn chat_complete(&self, system: &str, user: &str) -> Result<String, RagError> {
        #[derive(Serialize)]
        struct Req<'a> {
            model: &'a str,
            messages: Vec<Msg<'a>>,
        }
        self.chat(
            Stage::Generation,
            &Req {
                model: &self.chat_model,
                messages: vec![
                    Msg {
                        role: "system",
                        content: system,
                    },
                    Msg {
                        role: "user",
                        content: user,
                    },
                ],
            },
        )
        .await
    }

    /// JSON-mode chat completion, used by the planner. Failures (including
    /// unparseable JSON) are [`RagError::PlanningFailed`].
    pub async fn chat_json<T: for<'de> Deserialize<'de>>(
        &self,
        system: &str,
        user: &str,
    ) -> Result<T, RagError> {
        #[derive(Serialize)]
        struct Req<'a> {
            model: &'a str,
            messages: Vec<Msg<'a>>,
            response_format: RespFmt,
        }
        #[derive(Serialize)]
        struct RespFmt {
            r#type: &'static str,
        }
        let content = self
            .chat(
                Stage::Planning,
                &Req {
                    model: &self.chat_model,
                    messages: vec![
                        Msg {
                            role: "system",
                            content: system,
                        },
                        Msg {
                            role: "user",
                            content: user,
                        },
                    ],
                    response_format: RespFmt {
                        r#type: "json_object",
                    },
                },
            )
            .await?;
        serde_json::from_str::<T>(&content).map_err(|e| {
            RagError::failed(
                Stage::Planning,
                UpstreamError::InvalidResponse(format!("model returned invalid JSON: {e}")),
            )
        })
    }
}

/// Splits `texts` into consecutive index ranges for [`OpenAiClient::embed_batch`]: each
/// range holds at most `max_inputs` texts and at most `max_chars` characters in total (a
/// rough stand-in for the per-request token limit). A single text longer than `max_chars`
/// gets a range of its own. Limits below 1 are treated as 1.
pub fn embedding_batches(
    texts: &[String],
    max_inputs: usize,
    max_chars: usize,
) -> Vec<std::ops::Range<usize>> {
    let (max_inputs, max_chars) = (max_inputs.max(1), max_chars.max(1));
    let mut batches = Vec::new();
    let (mut start, mut chars) = (0, 0);
    for (i, text) in texts.iter().enumerate() {
        let len = text.chars().count();
        if i > start && (i - start >= max_inputs || chars + len > max_chars) {
            batches.push(start..i);
            (start, chars) = (i, 0);
        }
        chars += len;
    }
    if start < texts.len() {
        batches.push(start..texts.len());
    }
    batches
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{EnvVarGuard, lock_env};
    use wiremock::matchers::{body_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn mock_client(base_url: String, retry: RetryPolicy) -> OpenAiClient {
        OpenAiClient {
            api_key: "test_key".to_string(),
            embedding_api_key: "test_key".to_string(),
            embedding_base_url: base_url.clone(),
            base_url,
            embedding_model: "test-model".to_string(),
            chat_model: "test-chat".to_string(),
            http: reqwest::Client::new(),
            retry,
        }
    }

    fn texts(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn embedding_batches_respect_input_and_char_limits() {
        let t = texts(&["aaaa", "bb", "cccccc", "d", "e", "ffffffffff", "g"]);
        // At most 3 inputs and 8 chars per batch; the 10-char text is alone.
        let batches = embedding_batches(&t, 3, 8);
        assert_eq!(batches, [0..2, 2..5, 5..6, 6..7]);
        for b in &batches {
            let chars: usize = t[b.clone()].iter().map(|s| s.len()).sum();
            assert!(b.len() <= 3);
            assert!(chars <= 8 || b.len() == 1);
        }
        // Every text lands in exactly one batch, in order.
        let flat: Vec<usize> = batches.into_iter().flatten().collect();
        assert_eq!(flat, (0..t.len()).collect::<Vec<_>>());

        assert_eq!(embedding_batches(&t, 100, 1000), vec![0..7]);
        assert_eq!(embedding_batches(&t, 0, 0).len(), t.len());
        assert!(embedding_batches(&[], 3, 8).is_empty());
        // Characters, not bytes.
        assert_eq!(
            embedding_batches(&texts(&["åäö", "åäö"]), 10, 6),
            vec![0..2]
        );
    }

    #[tokio::test]
    async fn embed_batch_sends_array_input_and_maps_by_index() {
        use wiremock::matchers::{body_json, header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .and(header("authorization", "Bearer test_key"))
            .and(body_json(serde_json::json!({
                "input": ["first", "second", "third"],
                "model": "test-model"
            })))
            // Out of order: the index field, not the position, decides.
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [
                    {"index": 2, "embedding": [3.0]},
                    {"index": 0, "embedding": [1.0]},
                    {"index": 1, "embedding": [2.0]}
                ]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = mock_client(server.uri(), RetryPolicy::none());
        let out = client
            .embed_batch(&texts(&["first", "second", "third"]))
            .await
            .unwrap();
        assert_eq!(out, [vec![1.0], vec![2.0], vec![3.0]]);
        assert!(client.embed_batch(&[]).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn embed_batch_rejects_missing_duplicate_or_out_of_range_indexes() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        for data in [
            serde_json::json!([{"index": 0, "embedding": [1.0]}]),
            serde_json::json!([{"index": 0, "embedding": [1.0]}, {"index": 0, "embedding": [2.0]}]),
            serde_json::json!([{"index": 0, "embedding": [1.0]}, {"index": 5, "embedding": [2.0]}]),
            serde_json::json!([{"index": 0, "embedding": [1.0]}, {"index": 1, "embedding": []}]),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/v1/embeddings"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(serde_json::json!({"data": data})),
                )
                .mount(&server)
                .await;
            let client = mock_client(server.uri(), RetryPolicy::none());
            let err = client
                .embed_batch(&texts(&["a", "b"]))
                .await
                .expect_err(&data.to_string());
            assert!(matches!(err, RagError::EmbeddingFailed { .. }), "{err:?}");
        }
    }

    #[tokio::test]
    async fn embed_batch_retries_transient_failures() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(ResponseTemplate::new(429).insert_header("retry-after-ms", "1"))
            .up_to_n_times(1)
            .with_priority(1)
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{"index": 0, "embedding": [0.5]}]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let retry = RetryPolicy {
            max_attempts: 3,
            base_delay: std::time::Duration::from_millis(1),
            max_delay: std::time::Duration::from_millis(50),
        };
        let client = mock_client(server.uri(), retry);
        assert_eq!(
            client.embed_batch(&texts(&["a"])).await.unwrap(),
            [vec![0.5]]
        );
    }

    #[tokio::test]
    async fn embed_batch_does_not_retry_client_errors() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(ResponseTemplate::new(400))
            .expect(1)
            .mount(&server)
            .await;
        let retry = RetryPolicy {
            max_attempts: 3,
            base_delay: std::time::Duration::from_millis(1),
            max_delay: std::time::Duration::from_millis(50),
        };
        let err = mock_client(server.uri(), retry)
            .embed_batch(&texts(&["a"]))
            .await
            .unwrap_err();
        assert!(matches!(err, RagError::EmbeddingFailed { .. }), "{err:?}");
    }

    #[test]
    fn test_openai_client_new_from_env_defaults() {
        let _env_lock = lock_env();

        unsafe {
            std::env::remove_var("OPENAI_BASE_URL");
            std::env::remove_var("OPENAI_EMBEDDING_MODEL");
            std::env::remove_var("OPENAI_CHAT_MODEL");
            std::env::remove_var("OPENAI_EMBEDDING_BASE_URL");
            std::env::remove_var("OPENAI_EMBEDDING_API_KEY");
            std::env::set_var("OPENAI_API_KEY", "test_key");
        }

        let client = OpenAiClient::new_from_env().unwrap();
        assert_eq!(client.api_key, "test_key");
        assert_eq!(client.base_url, "https://api.openai.com");
        assert_eq!(client.embedding_model, "text-embedding-3-small");
        assert_eq!(client.chat_model, "o4-mini");
        // Embeddings use the same endpoint and key unless configured separately.
        assert_eq!(client.embedding_base_url, "https://api.openai.com");
        assert_eq!(client.embedding_api_key, "test_key");

        unsafe {
            std::env::remove_var("OPENAI_API_KEY");
        }
    }

    #[test]
    fn test_openai_client_new_from_env_custom() {
        let _env_lock = lock_env();

        unsafe {
            std::env::set_var("OPENAI_API_KEY", "custom_key");
            std::env::set_var("OPENAI_BASE_URL", "https://custom.openai.com");
            std::env::set_var("OPENAI_EMBEDDING_MODEL", "custom-embedding-model");
            std::env::set_var("OPENAI_CHAT_MODEL", "custom-chat-model");
        }

        let client = OpenAiClient::new_from_env().unwrap();
        assert_eq!(client.api_key, "custom_key");
        assert_eq!(client.base_url, "https://custom.openai.com");
        assert_eq!(client.embedding_model, "custom-embedding-model");
        assert_eq!(client.chat_model, "custom-chat-model");
        assert_eq!(client.embedding_base_url, "https://custom.openai.com");
        assert_eq!(client.embedding_api_key, "custom_key");

        // Cleanup
        unsafe {
            std::env::remove_var("OPENAI_API_KEY");
            std::env::remove_var("OPENAI_BASE_URL");
            std::env::remove_var("OPENAI_EMBEDDING_MODEL");
            std::env::remove_var("OPENAI_CHAT_MODEL");
        }
    }

    #[test]
    fn new_from_env_reads_a_separate_embedding_endpoint_and_key() {
        let _env_lock = lock_env();
        let _guards = [
            "OPENAI_API_KEY",
            "OPENAI_BASE_URL",
            "OPENAI_EMBEDDING_BASE_URL",
            "OPENAI_EMBEDDING_API_KEY",
        ]
        .map(EnvVarGuard::preserve);

        unsafe {
            std::env::set_var("OPENAI_API_KEY", "chat_key");
            std::env::set_var("OPENAI_BASE_URL", "https://chat.example");
            std::env::set_var("OPENAI_EMBEDDING_BASE_URL", " http://tei:8080 ");
            std::env::set_var("OPENAI_EMBEDDING_API_KEY", "embed_key");
        }
        let client = OpenAiClient::new_from_env().unwrap();
        assert_eq!(client.base_url, "https://chat.example");
        assert_eq!(client.api_key, "chat_key");
        assert_eq!(client.embedding_base_url, "http://tei:8080");
        assert_eq!(client.embedding_api_key, "embed_key");

        // Blank values fall back like unset ones; the URL and key are independent.
        unsafe {
            std::env::set_var("OPENAI_EMBEDDING_BASE_URL", "http://tei:8080");
            std::env::set_var("OPENAI_EMBEDDING_API_KEY", "  ");
        }
        let client = OpenAiClient::new_from_env().unwrap();
        assert_eq!(client.embedding_base_url, "http://tei:8080");
        assert_eq!(client.embedding_api_key, "chat_key");
        unsafe {
            std::env::set_var("OPENAI_EMBEDDING_BASE_URL", "");
            std::env::set_var("OPENAI_EMBEDDING_API_KEY", "embed_key");
        }
        let client = OpenAiClient::new_from_env().unwrap();
        assert_eq!(client.embedding_base_url, "https://chat.example");
        assert_eq!(client.embedding_api_key, "embed_key");
    }

    #[test]
    fn test_openai_client_new_from_env_missing_api_key() {
        let _env_lock = lock_env();
        let _api_key_guard = EnvVarGuard::preserve("OPENAI_API_KEY");

        unsafe {
            std::env::remove_var("OPENAI_API_KEY");
        }

        let result = OpenAiClient::new_from_env();
        assert!(result.is_err());
    }

    fn client(server: &MockServer) -> OpenAiClient {
        mock_client(server.uri(), RetryPolicy::none())
    }

    #[tokio::test]
    async fn embeddings_go_to_the_embedding_endpoint_with_its_key() {
        let chat = MockServer::start().await;
        let embeddings = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .and(header("authorization", "Bearer embed_key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                serde_json::json!({"data": [{"index": 0, "embedding": [0.5, 0.25]}]}),
            ))
            .expect(2)
            .mount(&embeddings)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(header("authorization", "Bearer test_key"))
            .respond_with(chat_reply(serde_json::json!("hi")))
            .expect(1)
            .mount(&chat)
            .await;
        let mut c = client(&chat);
        c.embedding_base_url = embeddings.uri();
        c.embedding_api_key = "embed_key".into();
        assert_eq!(c.embed("a").await.unwrap(), [0.5, 0.25]);
        assert_eq!(
            c.embed_batch(&texts(&["a"])).await.unwrap(),
            [vec![0.5, 0.25]]
        );
        assert_eq!(c.chat_complete("s", "u").await.unwrap(), "hi");
    }

    fn chat_reply(content: serde_json::Value) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "choices": [{"message": {"content": content}}]
        }))
    }

    #[tokio::test]
    async fn embed_sends_model_input_and_bearer_key() {
        let server = MockServer::start().await;
        // The mock only matches the documented request shape.
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .and(header("authorization", "Bearer test_key"))
            .and(body_json(
                serde_json::json!({"input": "Återförsök 🚀", "model": "test-model"}),
            ))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"data": [{"embedding": [0.1, 0.2, 0.3]}]})),
            )
            .expect(1)
            .mount(&server)
            .await;

        let embedding = client(&server).embed("Återförsök 🚀").await.unwrap();
        assert_eq!(embedding, vec![0.1, 0.2, 0.3]);
    }

    #[tokio::test]
    async fn embed_failures_are_embedding_errors_never_empty_vectors() {
        for (status, body) in [
            (401, serde_json::json!({"error": "bad key"})),
            (200, serde_json::json!({"data": []})),
            (200, serde_json::json!({"data": [{"embedding": []}]})),
            (200, serde_json::json!({"unexpected": true})),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/v1/embeddings"))
                .respond_with(ResponseTemplate::new(status).set_body_json(body.clone()))
                .mount(&server)
                .await;
            let err = client(&server).embed("text").await.unwrap_err();
            assert!(
                matches!(err, RagError::EmbeddingFailed { .. }),
                "{status} {body}: {err:?}"
            );
        }
    }

    #[tokio::test]
    async fn chat_complete_sends_system_and_user_messages() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(header("authorization", "Bearer test_key"))
            .and(body_json(serde_json::json!({
                "model": "test-chat",
                "messages": [
                    {"role": "system", "content": "You are a helpful assistant"},
                    {"role": "user", "content": "Hello"}
                ]
            })))
            .respond_with(chat_reply(serde_json::json!("This is a test response")))
            .expect(1)
            .mount(&server)
            .await;

        let answer = client(&server)
            .chat_complete("You are a helpful assistant", "Hello")
            .await
            .unwrap();
        assert_eq!(answer, "This is a test response");
    }

    #[tokio::test]
    async fn chat_json_requests_json_mode_and_parses_the_content() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(body_json(serde_json::json!({
                "model": "test-chat",
                "messages": [
                    {"role": "system", "content": "plan"},
                    {"role": "user", "content": "Get JSON"}
                ],
                "response_format": {"type": "json_object"}
            })))
            .respond_with(chat_reply(serde_json::json!("{\"result\": \"parsed\"}")))
            .expect(1)
            .mount(&server)
            .await;

        let json = client(&server)
            .chat_json::<serde_json::Value>("plan", "Get JSON")
            .await
            .unwrap();
        assert_eq!(json, serde_json::json!({"result": "parsed"}));
    }

    #[tokio::test]
    async fn chat_json_rejects_non_json_and_empty_content_as_planning_failures() {
        for content in [
            serde_json::json!("not json"),
            serde_json::json!(""),
            serde_json::Value::Null,
        ] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/v1/chat/completions"))
                .respond_with(chat_reply(content.clone()))
                .mount(&server)
                .await;
            let err = client(&server)
                .chat_json::<serde_json::Value>("s", "u")
                .await
                .unwrap_err();
            assert!(
                matches!(err, RagError::PlanningFailed { .. }),
                "{content}: {err:?}"
            );
        }
    }
}
