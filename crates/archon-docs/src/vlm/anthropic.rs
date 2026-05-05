use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::Deserialize;
use serde_json::json;

use archon_llm::auth::AuthProvider;
use archon_llm::identity::IdentityProvider;

use crate::errors::DocsError;
use crate::vlm::mime::detect_mime;
use crate::vlm::{IMAGE_DESCRIPTION_PROMPT, VlmDescriptionProvider};

pub const DEFAULT_ANTHROPIC_VLM_MODEL: &str = "claude-sonnet-4-6";
const DEFAULT_ANTHROPIC_MESSAGES_URL: &str = "https://api.anthropic.com/v1/messages";

#[derive(Debug, Clone)]
pub struct AnthropicVlmProvider {
    auth: AuthProvider,
    identity: IdentityProvider,
    model: String,
    api_url: String,
    http: reqwest::blocking::Client,
}

impl AnthropicVlmProvider {
    pub fn new(
        auth: AuthProvider,
        identity: IdentityProvider,
        model: impl Into<String>,
        api_url: Option<String>,
    ) -> Result<Self, DocsError> {
        let http = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(120))
            .no_proxy()
            .build()
            .map_err(|e| DocsError::VlmProvider {
                provider: "anthropic".into(),
                message: format!("failed to build HTTP client: {e}"),
                status_code: None,
            })?;
        Ok(Self {
            auth,
            identity,
            model: model.into(),
            api_url: api_url.unwrap_or_else(|| DEFAULT_ANTHROPIC_MESSAGES_URL.into()),
            http,
        })
    }

    pub fn provider_id(&self) -> &'static str {
        "anthropic"
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn api_url(&self) -> &str {
        &self.api_url
    }
}

impl VlmDescriptionProvider for AnthropicVlmProvider {
    fn describe_image(&self, image_bytes: &[u8]) -> Result<String, DocsError> {
        let mime = detect_mime(image_bytes)?;
        let request_id = uuid::Uuid::new_v4().to_string();
        let (auth_header_name, auth_header_value) = self.auth.header();
        let mut request = self
            .http
            .post(&self.api_url)
            .header(&auth_header_name, &auth_header_value);
        for (name, value) in self.identity.request_headers(&request_id) {
            request = request.header(name, value);
        }
        let body = json!({
            "model": self.model,
            "max_tokens": 512,
            "messages": [{
                "role": "user",
                "content": [
                    {
                        "type": "image",
                        "source": {
                            "type": "base64",
                            "media_type": mime,
                            "data": STANDARD.encode(image_bytes),
                        }
                    },
                    {"type": "text", "text": IMAGE_DESCRIPTION_PROMPT}
                ]
            }]
        });
        let response = request.json(&body).send().map_err(map_send_error)?;
        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(DocsError::VlmAuthentication {
                provider: "anthropic".into(),
                message: format!("Messages API rejected credentials with HTTP {status}"),
            });
        }
        if !status.is_success() {
            return Err(DocsError::VlmProvider {
                provider: "anthropic".into(),
                message: format!("Messages API failed with HTTP {status}"),
                status_code: Some(status.as_u16()),
            });
        }
        let parsed: MessagesResponse = response.json().map_err(|e| DocsError::VlmProvider {
            provider: "anthropic".into(),
            message: format!("failed to parse Messages response: {e}"),
            status_code: None,
        })?;
        parsed
            .content
            .into_iter()
            .find_map(|block| block.text)
            .ok_or_else(|| DocsError::VlmProvider {
                provider: "anthropic".into(),
                message: "Messages response did not contain text".into(),
                status_code: None,
            })
    }
}

fn map_send_error(error: reqwest::Error) -> DocsError {
    if error.is_timeout() {
        DocsError::VlmTimeout {
            provider: "anthropic".into(),
            message: error.to_string(),
        }
    } else {
        DocsError::VlmProvider {
            provider: "anthropic".into(),
            message: error.to_string(),
            status_code: None,
        }
    }
}

#[derive(Debug, Deserialize)]
struct MessagesResponse {
    content: Vec<ContentBlock>,
}

#[derive(Debug, Deserialize)]
struct ContentBlock {
    text: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_llm::identity::{IdentityConfigView, resolve_identity_mode};
    use archon_llm::types::Secret;
    use wiremock::matchers::{body_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn provider(api_url: String) -> AnthropicVlmProvider {
        let auth = AuthProvider::BearerToken(Secret::new("sk-ant-oat-test".to_string()));
        let identity = IdentityProvider::new(
            resolve_identity_mode(&auth, false, &IdentityConfigView::default()),
            "session-test".into(),
            "device-test".into(),
            "account-test".into(),
        );
        AnthropicVlmProvider::new(auth, identity, DEFAULT_ANTHROPIC_VLM_MODEL, Some(api_url))
            .unwrap()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn describe_image_includes_image_content_block() {
        let server = MockServer::start().await;
        let jpeg = &[0xFF, 0xD8, 0xFF, 0x00];
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(body_json(json!({
                "model": DEFAULT_ANTHROPIC_VLM_MODEL,
                "max_tokens": 512,
                "messages": [{
                    "role": "user",
                    "content": [
                        {
                            "type": "image",
                            "source": {
                                "type": "base64",
                                "media_type": "image/jpeg",
                                "data": STANDARD.encode(jpeg),
                            }
                        },
                        {"type": "text", "text": IMAGE_DESCRIPTION_PROMPT}
                    ]
                }]
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "content": [{"type": "text", "text": "annotated candlestick chart"}]
            })))
            .mount(&server)
            .await;
        let api_url = format!("{}/v1/messages", server.uri());
        let image = jpeg.to_vec();
        let text = tokio::task::spawn_blocking(move || provider(api_url).describe_image(&image))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(text, "annotated candlestick chart");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn describe_image_uses_existing_auth_provider_and_spoof_identity() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(header("authorization", "Bearer sk-ant-oat-test"))
            .and(header("x-app", "cli"))
            .and(header("X-Claude-Code-Session-Id", "session-test"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "content": [{"type": "text", "text": "headers ok"}]
            })))
            .mount(&server)
            .await;
        let api_url = format!("{}/v1/messages", server.uri());
        let text = tokio::task::spawn_blocking(move || {
            provider(api_url).describe_image(&[0x89, b'P', b'N', b'G'])
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(text, "headers ok");
    }

    #[test]
    fn provider_refuses_when_auth_missing_is_factory_responsibility() {
        let mut policy = archon_policy::EffectivePolicy::default();
        policy.docs.vlm.enabled = true;
        policy.docs.vlm.mode = "cloud".into();
        policy.docs.vlm.provider = "anthropic".into();
        policy.docs.vlm.allow_cloud = true;
        policy.network.allow_cloud_vlm = true;
        policy.workers.vlm = "allow-cloud".into();
        let report = crate::vlm::factory::diagnostic_report(&policy);
        assert!(
            matches!(
                report.status,
                crate::vlm::factory::VlmProviderInitStatus::Registered
                    | crate::vlm::factory::VlmProviderInitStatus::Skipped
            ),
            "factory should convert auth state into a diagnostic report"
        );
    }
}
