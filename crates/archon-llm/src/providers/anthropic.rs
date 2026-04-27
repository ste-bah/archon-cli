/// `AnthropicProvider` — wraps `AnthropicClient` and implements `LlmProvider`.
///
/// Converts between the provider-agnostic `LlmRequest` / `LlmResponse` types
/// and the Anthropic-specific `MessageRequest` / `StreamEvent` types without
/// modifying the underlying `AnthropicClient`.
use async_trait::async_trait;
use tokio::sync::mpsc::Receiver;

use crate::anthropic::{AnthropicClient, ApiError};
use crate::provider::{LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo, ProviderFeature};
use crate::streaming::StreamEvent;

// ---------------------------------------------------------------------------
// Error conversion
// ---------------------------------------------------------------------------

impl From<ApiError> for LlmError {
    fn from(e: ApiError) -> Self {
        match e {
            ApiError::HttpError(msg) => LlmError::Http(msg),
            ApiError::AuthError(msg) => LlmError::Auth(msg),
            ApiError::RateLimited {
                retry_after_secs,
                body_preview,
            } => LlmError::RateLimited {
                retry_after_secs,
                body_preview,
            },
            ApiError::Overloaded => LlmError::Overloaded,
            ApiError::ServerError { status, message } => LlmError::Server { status, message },
            ApiError::SerializeError(msg) => LlmError::Serialize(msg),
        }
    }
}

// ---------------------------------------------------------------------------
// AnthropicProvider
// ---------------------------------------------------------------------------

/// An `LlmProvider` backed by `AnthropicClient`.
///
/// The inner client remains accessible via `client()` for code paths that
/// need Anthropic-specific accessors (auth headers, identity headers).
pub struct AnthropicProvider {
    client: AnthropicClient,
}

impl AnthropicProvider {
    /// Wrap an existing `AnthropicClient`.
    pub fn new(client: AnthropicClient) -> Self {
        Self { client }
    }

    /// Access the underlying `AnthropicClient` directly.
    ///
    /// Used to reach `auth()` and `identity()` for header injection in code
    /// paths that must remain Anthropic-aware.
    pub fn client(&self) -> &AnthropicClient {
        &self.client
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![
            ModelInfo {
                id: "claude-opus-4-6".into(),
                display_name: "Claude Opus 4.6".into(),
                context_window: 200_000,
            },
            ModelInfo {
                id: "claude-sonnet-4-6".into(),
                display_name: "Claude Sonnet 4.6".into(),
                context_window: 200_000,
            },
            ModelInfo {
                id: "claude-haiku-4-5-20251001".into(),
                display_name: "Claude Haiku 4.5".into(),
                context_window: 200_000,
            },
        ]
    }

    async fn stream(&self, request: LlmRequest) -> Result<Receiver<StreamEvent>, LlmError> {
        let msg_request = request.into();
        self.client
            .stream_message(msg_request)
            .await
            .map_err(LlmError::from)
    }

    /// Collect a full non-streaming response by consuming all stream events.
    ///
    /// Drives the same `stream_message` path underneath and collects
    /// `TextDelta` + usage tokens into an `LlmResponse`.
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse, LlmError> {
        use crate::streaming::StreamEvent;
        use crate::types::Usage;

        let mut rx = self.stream(request).await?;

        let mut text_parts: Vec<String> = Vec::new();
        let mut usage = Usage::default();
        let mut stop_reason = String::new();

        while let Some(event) = rx.recv().await {
            match event {
                StreamEvent::MessageStart {
                    usage: start_usage, ..
                } => {
                    usage.merge(&start_usage);
                }
                StreamEvent::TextDelta { text, .. } => {
                    text_parts.push(text);
                }
                StreamEvent::MessageDelta {
                    stop_reason: sr,
                    usage: delta_usage,
                } => {
                    if let Some(sr) = sr {
                        stop_reason = sr;
                    }
                    if let Some(u) = delta_usage {
                        usage.merge(&u);
                    }
                }
                _ => {}
            }
        }

        let full_text = text_parts.join("");
        let content = if full_text.is_empty() {
            vec![]
        } else {
            vec![serde_json::json!({"type": "text", "text": full_text})]
        };

        Ok(LlmResponse {
            content,
            usage,
            stop_reason,
        })
    }

    fn supports_feature(&self, feature: ProviderFeature) -> bool {
        matches!(
            feature,
            ProviderFeature::Thinking
                | ProviderFeature::ToolUse
                | ProviderFeature::PromptCaching
                | ProviderFeature::Vision
                | ProviderFeature::SystemPrompt
                | ProviderFeature::Streaming
        )
    }

    fn as_anthropic(&self) -> Option<&AnthropicClient> {
        Some(&self.client)
    }
}
