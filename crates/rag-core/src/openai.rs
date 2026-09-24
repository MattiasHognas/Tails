use crate::error::{RagError, Stage, UpstreamError};
use crate::resilience::{HttpConfig, RetryPolicy, send_with_retry};
use anyhow::Result;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

#[derive(Debug, Clone)]
pub struct OpenAiClient {
    pub api_key: String,
    pub base_url: String,
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
            api_key,
            base_url,
            embedding_model,
            chat_model,
            http: HttpConfig::default().build_client(),
            retry: RetryPolicy::default(),
        }
    }

    pub fn new_from_env() -> Result<Self> {
        let mut client = Self::new(
            std::env::var("OPENAI_API_KEY")?,
            std::env::var("OPENAI_BASE_URL").unwrap_or_else(|_| "https://api.openai.com".into()),
            std::env::var("OPENAI_EMBEDDING_MODEL")
                .unwrap_or_else(|_| "text-embedding-3-small".into()),
            std::env::var("OPENAI_CHAT_MODEL").unwrap_or_else(|_| "o4-mini".into()),
        );
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
        let url = format!("{}{}", self.base_url, path);
        let what = format!("openai {path}");
        let r = send_with_retry(&self.retry, &what, || {
            self.http.post(&url).bearer_auth(&self.api_key).json(body)
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
                "/v1/embeddings",
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{EnvVarGuard, lock_env};

    #[test]
    fn test_openai_client_new_from_env_defaults() {
        let _env_lock = lock_env();

        unsafe {
            std::env::remove_var("OPENAI_BASE_URL");
            std::env::remove_var("OPENAI_EMBEDDING_MODEL");
            std::env::remove_var("OPENAI_CHAT_MODEL");
            std::env::set_var("OPENAI_API_KEY", "test_key");
        }

        let client = OpenAiClient::new_from_env().unwrap();
        assert_eq!(client.api_key, "test_key");
        assert_eq!(client.base_url, "https://api.openai.com");
        assert_eq!(client.embedding_model, "text-embedding-3-small");
        assert_eq!(client.chat_model, "o4-mini");

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

        // Cleanup
        unsafe {
            std::env::remove_var("OPENAI_API_KEY");
            std::env::remove_var("OPENAI_BASE_URL");
            std::env::remove_var("OPENAI_EMBEDDING_MODEL");
            std::env::remove_var("OPENAI_CHAT_MODEL");
        }
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

    #[test]
    fn test_url_formatting() {
        let client = OpenAiClient {
            api_key: "test_key".to_string(),
            base_url: "https://api.openai.com".to_string(),
            embedding_model: "text-embedding-3-small".to_string(),
            chat_model: "o4-mini".to_string(),
            http: reqwest::Client::new(),
            retry: RetryPolicy::none(),
        };

        let embed_url = format!("{}/v1/embeddings", client.base_url);
        assert_eq!(embed_url, "https://api.openai.com/v1/embeddings");

        let chat_url = format!("{}/v1/chat/completions", client.base_url);
        assert_eq!(chat_url, "https://api.openai.com/v1/chat/completions");
    }

    #[test]
    fn test_openai_client_custom_base_url() {
        let _env_lock = lock_env();

        unsafe {
            std::env::set_var("OPENAI_API_KEY", "test");
            std::env::set_var("OPENAI_BASE_URL", "https://custom-llm-gateway.example.com");
        }

        let client = OpenAiClient::new_from_env().unwrap();
        assert_eq!(client.base_url, "https://custom-llm-gateway.example.com");

        unsafe {
            std::env::remove_var("OPENAI_API_KEY");
            std::env::remove_var("OPENAI_BASE_URL");
        }
    }

    #[tokio::test]
    async fn test_embed_with_mock_server() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{
                    "embedding": [0.1, 0.2, 0.3]
                }]
            })))
            .mount(&mock_server)
            .await;

        let client = OpenAiClient {
            api_key: "test_key".to_string(),
            base_url: mock_server.uri(),
            embedding_model: "test-model".to_string(),
            chat_model: "test-chat".to_string(),
            http: reqwest::Client::new(),
            retry: RetryPolicy::none(),
        };

        let result = client.embed("test text").await;
        assert!(result.is_ok());
        let embedding = result.unwrap();
        assert_eq!(embedding, vec![0.1, 0.2, 0.3]);
    }

    #[tokio::test]
    async fn test_embed_error_handling() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&mock_server)
            .await;

        let client = OpenAiClient {
            api_key: "invalid_key".to_string(),
            base_url: mock_server.uri(),
            embedding_model: "test-model".to_string(),
            chat_model: "test-chat".to_string(),
            http: reqwest::Client::new(),
            retry: RetryPolicy::none(),
        };

        let result = client.embed("test text").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_chat_complete_with_mock_server() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{
                    "message": {
                        "content": "This is a test response"
                    }
                }]
            })))
            .mount(&mock_server)
            .await;

        let client = OpenAiClient {
            api_key: "test_key".to_string(),
            base_url: mock_server.uri(),
            embedding_model: "test-model".to_string(),
            chat_model: "test-chat".to_string(),
            http: reqwest::Client::new(),
            retry: RetryPolicy::none(),
        };

        let result = client
            .chat_complete("You are a helpful assistant", "Hello")
            .await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "This is a test response");
    }

    #[tokio::test]
    async fn test_chat_json_with_mock_server() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{
                    "message": {
                        "content": "{\"result\": \"parsed\"}"
                    }
                }]
            })))
            .mount(&mock_server)
            .await;

        let client = OpenAiClient {
            api_key: "test_key".to_string(),
            base_url: mock_server.uri(),
            embedding_model: "test-model".to_string(),
            chat_model: "test-chat".to_string(),
            http: reqwest::Client::new(),
            retry: RetryPolicy::none(),
        };

        let result = client
            .chat_json::<serde_json::Value>("You are a helpful assistant", "Get JSON")
            .await;
        assert!(result.is_ok());
        let json = result.unwrap();
        assert_eq!(json["result"], "parsed");
    }
}
