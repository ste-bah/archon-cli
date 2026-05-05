use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::Deserialize;
use serde_json::json;

use crate::errors::DocsError;
use crate::vlm::{IMAGE_DESCRIPTION_PROMPT, VlmDescriptionProvider};

pub const DEFAULT_OLLAMA_ENDPOINT: &str = "http://localhost:11434";
pub const DEFAULT_OLLAMA_MODEL: &str = "gemma4:e4b";

#[derive(Clone)]
pub struct OllamaVlmProvider {
    endpoint: String,
    model: String,
    http: reqwest::blocking::Client,
}

impl OllamaVlmProvider {
    pub fn new(
        endpoint: impl Into<String>,
        model: impl Into<String>,
        timeout: Duration,
    ) -> Result<Self, DocsError> {
        let http = reqwest::blocking::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| DocsError::VlmProvider {
                provider: "ollama".into(),
                message: format!("failed to build HTTP client: {e}"),
                status_code: None,
            })?;
        Ok(Self {
            endpoint: endpoint.into(),
            model: model.into(),
            http,
        })
    }

    pub fn from_policy(policy: &archon_policy::OllamaVlmPolicy) -> Result<Self, DocsError> {
        Self::new(
            policy.endpoint.clone(),
            policy.model.clone(),
            Duration::from_secs(policy.timeout_secs),
        )
    }

    pub fn provider_id(&self) -> &'static str {
        "ollama"
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub fn health_check(&self) -> Result<(), DocsError> {
        let url = format!("{}/api/tags", self.endpoint.trim_end_matches('/'));
        let response = self
            .http
            .get(url)
            .send()
            .map_err(|e| self.map_send_error(e))?;
        let status = response.status();
        if !status.is_success() {
            return Err(DocsError::VlmProvider {
                provider: "ollama".into(),
                message: format!("health check failed with HTTP {status}"),
                status_code: Some(status.as_u16()),
            });
        }
        let tags: TagsResponse = response.json().map_err(|e| DocsError::VlmProvider {
            provider: "ollama".into(),
            message: format!("failed to parse /api/tags response: {e}"),
            status_code: None,
        })?;
        let found = tags.models.iter().any(|model| {
            model.name == self.model
                || model.model.as_deref() == Some(self.model.as_str())
                || model.name.strip_suffix(":latest") == Some(self.model.as_str())
        });
        if found {
            Ok(())
        } else {
            Err(DocsError::VlmProvider {
                provider: "ollama".into(),
                message: format!("model '{}' is not installed in Ollama", self.model),
                status_code: None,
            })
        }
    }

    fn map_send_error(&self, error: reqwest::Error) -> DocsError {
        if error.is_timeout() {
            DocsError::VlmTimeout {
                provider: "ollama".into(),
                message: error.to_string(),
            }
        } else {
            DocsError::VlmProvider {
                provider: "ollama".into(),
                message: error.to_string(),
                status_code: None,
            }
        }
    }
}

impl VlmDescriptionProvider for OllamaVlmProvider {
    fn describe_image(&self, image_bytes: &[u8]) -> Result<String, DocsError> {
        let url = format!("{}/api/chat", self.endpoint.trim_end_matches('/'));
        let image = STANDARD.encode(image_bytes);
        let body = json!({
            "model": self.model,
            "messages": [{
                "role": "user",
                "content": IMAGE_DESCRIPTION_PROMPT,
                "images": [image],
            }],
            "stream": false,
        });
        let response = self
            .http
            .post(url)
            .json(&body)
            .send()
            .map_err(|e| self.map_send_error(e))?;
        let status = response.status();
        if !status.is_success() {
            return Err(DocsError::VlmProvider {
                provider: "ollama".into(),
                message: format!("image description failed with HTTP {status}"),
                status_code: Some(status.as_u16()),
            });
        }
        let parsed: ChatResponse = response.json().map_err(|e| DocsError::VlmProvider {
            provider: "ollama".into(),
            message: format!("failed to parse /api/chat response: {e}"),
            status_code: None,
        })?;
        Ok(parsed.message.content)
    }
}

#[derive(Debug, Deserialize)]
struct TagsResponse {
    models: Vec<TagModel>,
}

#[derive(Debug, Deserialize)]
struct TagModel {
    name: String,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    message: ChatMessage,
}

#[derive(Debug, Deserialize)]
struct ChatMessage {
    content: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn provider(endpoint: String, model: &str) -> OllamaVlmProvider {
        OllamaVlmProvider::new(endpoint, model, Duration::from_secs(5)).unwrap()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn health_check_succeeds_with_model_in_list() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/tags"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "models": [{"name": "gemma4:e4b"}]
            })))
            .mount(&server)
            .await;
        let endpoint = server.uri();
        tokio::task::spawn_blocking(move || provider(endpoint, "gemma4:e4b").health_check())
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn health_check_fails_when_model_missing() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/tags"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "models": [{"name": "llava:13b"}]
            })))
            .mount(&server)
            .await;
        let endpoint = server.uri();
        let err =
            tokio::task::spawn_blocking(move || provider(endpoint, "gemma4:e4b").health_check())
                .await
                .unwrap()
                .unwrap_err();
        assert!(err.to_string().contains("not installed"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn describe_image_calls_chat_with_base64_image() {
        let server = MockServer::start().await;
        let encoded = STANDARD.encode(b"image-bytes");
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .and(body_json(json!({
                "model": "gemma4:e4b",
                "messages": [{
                    "role": "user",
                    "content": IMAGE_DESCRIPTION_PROMPT,
                    "images": [encoded],
                }],
                "stream": false,
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "message": {"content": "chart slopes upward"}
            })))
            .mount(&server)
            .await;
        let endpoint = server.uri();
        let text = tokio::task::spawn_blocking(move || {
            provider(endpoint, "gemma4:e4b").describe_image(b"image-bytes")
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(text, "chart slopes upward");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn describe_image_returns_response_text() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "message": {"content": "diagram has three labelled nodes"}
            })))
            .mount(&server)
            .await;
        let endpoint = server.uri();
        let text = tokio::task::spawn_blocking(move || {
            provider(endpoint, "gemma4:e4b").describe_image(b"png")
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(text, "diagram has three labelled nodes");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn describe_image_handles_500_as_vlm_provider_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        let endpoint = server.uri();
        let err = tokio::task::spawn_blocking(move || {
            provider(endpoint, "gemma4:e4b").describe_image(b"png")
        })
        .await
        .unwrap()
        .unwrap_err();
        assert!(matches!(
            err,
            DocsError::VlmProvider {
                status_code: Some(500),
                ..
            }
        ));
    }
}
