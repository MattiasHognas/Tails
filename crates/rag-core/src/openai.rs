use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct OpenAiClient {
    pub api_key: String,
    pub base_url: String,
    pub embedding_model: String,
    pub chat_model: String,
    pub http: reqwest::Client,
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
            http: reqwest::Client::new(),
        }
    }

    pub fn new_from_env() -> Result<Self> {
        Ok(Self::new(
            std::env::var("OPENAI_API_KEY")?,
            std::env::var("OPENAI_BASE_URL").unwrap_or_else(|_| "https://api.openai.com".into()),
            std::env::var("OPENAI_EMBEDDING_MODEL")
                .unwrap_or_else(|_| "text-embedding-3-small".into()),
            std::env::var("OPENAI_CHAT_MODEL").unwrap_or_else(|_| "o4-mini".into()),
        ))
    }

    pub async fn embed(&self, text: &str) -> Result<Vec<f32>> {
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
        let url = format!("{}/v1/embeddings", self.base_url);
        let r = self
            .http
            .post(url)
            .bearer_auth(&self.api_key)
            .json(&Req {
                input: text,
                model: &self.embedding_model,
            })
            .send()
            .await?;
        if !r.status().is_success() {
            return Err(anyhow!("openai embeddings status {}", r.status()));
        }
        let v: Resp = r.json().await?;
        Ok(v.data
            .into_iter()
            .next()
            .map(|d| d.embedding)
            .unwrap_or_default())
    }

    pub async fn chat_complete(&self, system: &str, user: &str) -> Result<String> {
        #[derive(Serialize)]
        struct Msg<'a> {
            role: &'a str,
            content: &'a str,
        }
        #[derive(Serialize)]
        struct Req<'a> {
            model: &'a str,
            messages: Vec<Msg<'a>>,
        }
        #[derive(Deserialize)]
        struct Resp {
            choices: Vec<Choice>,
        }
        #[derive(Deserialize)]
        struct Choice {
            message: Message,
        }
        #[derive(Deserialize)]
        struct Message {
            content: String,
        }
        let url = format!("{}/v1/chat/completions", self.base_url);
        let r = self
            .http
            .post(url)
            .bearer_auth(&self.api_key)
            .json(&Req {
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
            })
            .send()
            .await?;
        if !r.status().is_success() {
            return Err(anyhow!("openai chat status {}", r.status()));
        }
        let v: Resp = r.json().await?;
        Ok(v.choices
            .first()
            .map(|c| c.message.content.clone())
            .unwrap_or_default())
    }

    pub async fn chat_json<T: for<'de> Deserialize<'de>>(
        &self,
        system: &str,
        user: &str,
    ) -> Result<T> {
        #[derive(Serialize)]
        struct Msg<'a> {
            role: &'a str,
            content: &'a str,
        }
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
        #[derive(Deserialize)]
        struct Resp {
            choices: Vec<Choice>,
        }
        #[derive(Deserialize)]
        struct Choice {
            message: Message,
        }
        #[derive(Deserialize)]
        struct Message {
            content: String,
        }
        let url = format!("{}/v1/chat/completions", self.base_url);
        let r = self
            .http
            .post(url)
            .bearer_auth(&self.api_key)
            .json(&Req {
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
            })
            .send()
            .await?;
        if !r.status().is_success() {
            return Err(anyhow!("openai chat(json) status {}", r.status()));
        }
        let v: Resp = r.json().await?;
        let content = v
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .unwrap_or_else(|| "{}".into());
        Ok(serde_json::from_str::<T>(&content)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::lock_env;

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
        };

        let result = client
            .chat_json::<serde_json::Value>("You are a helpful assistant", "Get JSON")
            .await;
        assert!(result.is_ok());
        let json = result.unwrap();
        assert_eq!(json["result"], "parsed");
    }
}
