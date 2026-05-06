use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::errors::DocsError;
use crate::vlm::mime::detect_mime;
use crate::vlm::retry::{RateLimitRetry, retry_rate_limited};
use crate::vlm::{IMAGE_DESCRIPTION_PROMPT, VlmDescriptionProvider};

pub const DEFAULT_OPENAI_COMPAT_ENDPOINT: &str = "http://localhost:1234/v1";
pub const DEFAULT_OPENAI_COMPAT_MODEL: &str = "google/gemma-3-12b-it";
pub const DEFAULT_OPENAI_COMPAT_API_KEY_ENV: &str = "OPENAI_API_KEY";
pub const DEFAULT_OPENAI_COMPAT_MAX_TOKENS: u32 = 1024;
pub const DEFAULT_OPENAI_COMPAT_TEMPERATURE: f32 = 0.2;

#[derive(Debug, Clone)]
pub struct OpenAiCompatVlmProvider {
    endpoint: String,
    model: String,
    api_key_env: String,
    api_key: Option<String>,
    max_tokens: u32,
    temperature: f32,
    http: reqwest::blocking::Client,
}

impl OpenAiCompatVlmProvider {
    pub fn new(
        endpoint: impl Into<String>,
        model: impl Into<String>,
        api_key_env: impl Into<String>,
        api_key: Option<String>,
        timeout: Duration,
        max_tokens: u32,
        temperature: f32,
    ) -> Result<Self, DocsError> {
        let endpoint = endpoint.into();
        if endpoint.trim().is_empty() {
            return Err(DocsError::VlmProvider {
                provider: "openai-compat".into(),
                message: "OpenAI-compatible VLM endpoint is blank".into(),
                status_code: None,
            });
        }
        let model = model.into();
        if model.trim().is_empty() {
            return Err(DocsError::VlmProvider {
                provider: "openai-compat".into(),
                message: "OpenAI-compatible VLM model is blank".into(),
                status_code: None,
            });
        }
        let http = reqwest::blocking::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| DocsError::VlmProvider {
                provider: "openai-compat".into(),
                message: format!("failed to build HTTP client: {e}"),
                status_code: None,
            })?;
        Ok(Self {
            endpoint,
            model,
            api_key_env: api_key_env.into(),
            api_key: api_key.filter(|value| !value.trim().is_empty()),
            max_tokens,
            temperature,
            http,
        })
    }

    pub fn from_policy(
        policy: &archon_policy::OpenAiCompatVlmPolicy,
        api_key: Option<String>,
    ) -> Result<Self, DocsError> {
        Self::new(
            policy.endpoint.clone(),
            policy.model.clone(),
            policy.api_key_env.clone(),
            api_key,
            Duration::from_secs(policy.timeout_secs),
            policy.max_tokens,
            policy.temperature,
        )
    }

    pub fn provider_id(&self) -> &'static str {
        "openai-compat"
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub fn health_check(&self) -> Result<(), DocsError> {
        let url = format!("{}/models", self.endpoint.trim_end_matches('/'));
        let response = self
            .with_auth(self.http.get(url))
            .send()
            .map_err(map_send_error)?;
        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(self.auth_error(status, "models endpoint rejected credentials"));
        }
        if !status.is_success() {
            return Err(DocsError::VlmProvider {
                provider: "openai-compat".into(),
                message: format!("health check failed with HTTP {status}"),
                status_code: Some(status.as_u16()),
            });
        }
        let models: ModelsResponse = response.json().map_err(|e| DocsError::VlmProvider {
            provider: "openai-compat".into(),
            message: format!("failed to parse /models response: {e}"),
            status_code: None,
        })?;
        let found = models.data.iter().any(|model| model.id == self.model);
        if found {
            Ok(())
        } else {
            Err(DocsError::VlmProvider {
                provider: "openai-compat".into(),
                message: format!(
                    "model '{}' not returned by OpenAI-compatible /models endpoint",
                    self.model
                ),
                status_code: None,
            })
        }
    }

    fn generate_once(&self, image_bytes: &[u8]) -> Result<String, DocsError> {
        let mime = detect_mime(image_bytes)?;
        let data_url = format!("data:{mime};base64,{}", STANDARD.encode(image_bytes));
        let url = format!("{}/chat/completions", self.endpoint.trim_end_matches('/'));
        let body = json!({
            "model": self.model,
            "max_tokens": self.max_tokens,
            "temperature": self.temperature,
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "text", "text": IMAGE_DESCRIPTION_PROMPT},
                    {"type": "image_url", "image_url": {"url": data_url}}
                ]
            }]
        });
        let response = self
            .with_auth(self.http.post(url))
            .json(&body)
            .send()
            .map_err(map_send_error)?;
        let status = response.status();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(DocsError::VlmRateLimit {
                provider: "openai-compat".into(),
                retry_after_secs: retry_after_secs(&response),
                message: "OpenAI-compatible VLM returned HTTP 429".into(),
            });
        }
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(self.auth_error(status, "chat completions endpoint rejected credentials"));
        }
        if !status.is_success() {
            return Err(DocsError::VlmProvider {
                provider: "openai-compat".into(),
                message: format!("chat completions failed with HTTP {status}"),
                status_code: Some(status.as_u16()),
            });
        }
        let parsed: ChatCompletionsResponse =
            response.json().map_err(|e| DocsError::VlmProvider {
                provider: "openai-compat".into(),
                message: format!("failed to parse chat completions response: {e}"),
                status_code: None,
            })?;
        parsed
            .choices
            .first()
            .and_then(|choice| content_to_text(&choice.message.content))
            .ok_or_else(|| DocsError::VlmProvider {
                provider: "openai-compat".into(),
                message: "chat completions response did not contain text".into(),
                status_code: None,
            })
    }

    fn with_auth(
        &self,
        request: reqwest::blocking::RequestBuilder,
    ) -> reqwest::blocking::RequestBuilder {
        match self.api_key.as_deref() {
            Some(key) => request.bearer_auth(key),
            None => request,
        }
    }

    fn auth_error(&self, status: reqwest::StatusCode, context: &str) -> DocsError {
        let message = if self.api_key.is_some() {
            format!("{context} with HTTP {status}")
        } else {
            format!(
                "{context} with HTTP {status}; set {} for cloud OpenAI-compatible providers",
                self.api_key_env
            )
        };
        DocsError::VlmAuthentication {
            provider: "openai-compat".into(),
            message,
        }
    }
}

impl VlmDescriptionProvider for OpenAiCompatVlmProvider {
    fn describe_image(&self, image_bytes: &[u8]) -> Result<String, DocsError> {
        retry_rate_limited(RateLimitRetry::vlm_default(Duration::from_secs(1)), || {
            self.generate_once(image_bytes)
        })
    }
}

fn map_send_error(error: reqwest::Error) -> DocsError {
    if error.is_timeout() {
        DocsError::VlmTimeout {
            provider: "openai-compat".into(),
            message: error.to_string(),
        }
    } else {
        DocsError::VlmProvider {
            provider: "openai-compat".into(),
            message: error.to_string(),
            status_code: None,
        }
    }
}

fn retry_after_secs(response: &reqwest::blocking::Response) -> u64 {
    response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(1)
}

fn content_to_text(content: &Value) -> Option<String> {
    match content {
        Value::String(text) => non_empty(text),
        Value::Array(blocks) => {
            let text = blocks
                .iter()
                .filter_map(|block| block.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n");
            non_empty(&text)
        }
        _ => None,
    }
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    data: Vec<ModelRecord>,
}

#[derive(Debug, Deserialize)]
struct ModelRecord {
    id: String,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionsResponse {
    choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: ChatMessage,
}

#[derive(Debug, Deserialize)]
struct ChatMessage {
    content: Value,
}

#[cfg(test)]
#[path = "openai_compat_tests.rs"]
mod tests;
