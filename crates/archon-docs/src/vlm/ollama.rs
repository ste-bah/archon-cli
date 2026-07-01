use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::Deserialize;
use serde_json::json;

use crate::errors::DocsError;
use crate::vlm::retry::{RateLimitRetry, retry_vlm_transient};
use crate::vlm::{IMAGE_DESCRIPTION_PROMPT, VlmDescriptionProvider};

pub const DEFAULT_OLLAMA_ENDPOINT: &str = "http://localhost:11434";
pub const DEFAULT_OLLAMA_MODEL: &str = "gemma4:e4b";

#[derive(Clone)]
pub struct OllamaVlmProvider {
    endpoint: String,
    model: String,
    http: reqwest::blocking::Client,
    /// Constrained-VRAM request knobs (see `archon_policy::OllamaVlmPolicy`). `None` → the key is
    /// omitted from the request body, i.e. Ollama server defaults (identical to legacy behavior).
    num_ctx: Option<u32>,
    keep_alive: Option<String>,
    num_gpu: Option<u32>,
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
            num_ctx: None,
            keep_alive: None,
            num_gpu: None,
        })
    }

    pub fn from_policy(policy: &archon_policy::OllamaVlmPolicy) -> Result<Self, DocsError> {
        let mut provider = Self::new(
            policy.endpoint.clone(),
            policy.model.clone(),
            Duration::from_secs(policy.timeout_secs),
        )?;
        provider.num_ctx = policy.num_ctx;
        provider.keep_alive = policy.keep_alive.clone();
        provider.num_gpu = policy.num_gpu;
        Ok(provider)
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
    fn describe_image(
        &self,
        image_bytes: &[u8],
        prompt: Option<&str>,
    ) -> Result<String, DocsError> {
        retry_vlm_transient(RateLimitRetry::vlm_default(Duration::from_secs(5)), || {
            self.describe_image_once(image_bytes, prompt)
        })
    }
}

impl OllamaVlmProvider {
    fn describe_image_once(
        &self,
        image_bytes: &[u8],
        prompt: Option<&str>,
    ) -> Result<String, DocsError> {
        let prompt_text = prompt.unwrap_or(IMAGE_DESCRIPTION_PROMPT);
        let url = format!("{}/api/chat", self.endpoint.trim_end_matches('/'));
        let image = STANDARD.encode(image_bytes);
        let mut body = json!({
            "model": self.model,
            "messages": [{
                "role": "user",
                "content": prompt_text,
                "images": [image],
            }],
            "stream": false,
        });
        // Constrained-VRAM knobs: num_ctx/num_gpu nest under "options", keep_alive is top-level.
        // Each is omitted entirely when unset, so the body stays byte-identical to the default.
        let mut options = serde_json::Map::new();
        if let Some(n) = self.num_ctx {
            options.insert("num_ctx".into(), json!(n));
        }
        if let Some(n) = self.num_gpu {
            options.insert("num_gpu".into(), json!(n));
        }
        if !options.is_empty() {
            body["options"] = serde_json::Value::Object(options);
        }
        if let Some(ka) = &self.keep_alive {
            body["keep_alive"] = json!(ka);
        }
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
            provider(endpoint, "gemma4:e4b").describe_image(b"image-bytes", None)
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(text, "chart slopes upward");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn describe_image_injects_vram_knobs() {
        // num_ctx + num_gpu nest under "options"; keep_alive is top-level.
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
                "options": {"num_ctx": 2048, "num_gpu": 0},
                "keep_alive": "0",
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "message": {"content": "ok"}
            })))
            .mount(&server)
            .await;
        let endpoint = server.uri();
        let text = tokio::task::spawn_blocking(move || {
            let mut p = provider(endpoint, "gemma4:e4b");
            p.num_ctx = Some(2048);
            p.num_gpu = Some(0); // 0 = force CPU; must be SENT, distinct from unset
            p.keep_alive = Some("0".into());
            p.describe_image(b"image-bytes", None)
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(text, "ok");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn describe_image_keep_alive_only_omits_options() {
        // Only keep_alive set → body carries keep_alive but NO "options" key (exact-match proves it).
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
                "keep_alive": "5m",
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "message": {"content": "ok"}
            })))
            .mount(&server)
            .await;
        let endpoint = server.uri();
        tokio::task::spawn_blocking(move || {
            let mut p = provider(endpoint, "gemma4:e4b");
            p.keep_alive = Some("5m".into());
            p.describe_image(b"image-bytes", None)
        })
        .await
        .unwrap()
        .unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn describe_image_uses_prompt_override() {
        let server = MockServer::start().await;
        let encoded = STANDARD.encode(b"image-bytes");
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .and(body_json(json!({
                "model": "gemma4:e4b",
                "messages": [{
                    "role": "user",
                    "content": crate::vlm::VIDEO_FRAME_PROMPT,
                    "images": [encoded],
                }],
                "stream": false,
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "message": {"content": "frame has a slide"}
            })))
            .mount(&server)
            .await;
        let endpoint = server.uri();
        let text = tokio::task::spawn_blocking(move || {
            provider(endpoint, "gemma4:e4b")
                .describe_image(b"image-bytes", Some(crate::vlm::VIDEO_FRAME_PROMPT))
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(text, "frame has a slide");
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
            provider(endpoint, "gemma4:e4b").describe_image(b"png", None)
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
            provider(endpoint, "gemma4:e4b").describe_image(b"png", None)
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
