/// `AnthropicProvider` — wraps `AnthropicClient` and implements `LlmProvider`.
///
/// Converts between the provider-agnostic `LlmRequest` / `LlmResponse` types
/// and the Anthropic-specific `MessageRequest` / `StreamEvent` types without
/// modifying the underlying `AnthropicClient`.
use async_trait::async_trait;
use tokio::sync::mpsc::Receiver;

use crate::anthropic::{AnthropicClient, ApiError};
use crate::context_window::classify_context_window_error;
use crate::provider::{
    DataFlowClassification, LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo,
    ProviderFeature, classify_data_flow_endpoint,
};
use crate::streaming::StreamEvent;

// ---------------------------------------------------------------------------
// Error conversion
// ---------------------------------------------------------------------------

impl From<ApiError> for LlmError {
    fn from(e: ApiError) -> Self {
        match e {
            ApiError::HttpError(msg) => {
                classify_context_window_error(None, None, None, &msg, Some("anthropic"), None)
                    .unwrap_or(LlmError::Http(msg))
            }
            ApiError::AuthError(msg) => LlmError::Auth(msg),
            ApiError::RateLimited { retry_after_secs } => {
                LlmError::RateLimited { retry_after_secs }
            }
            ApiError::Overloaded => LlmError::Overloaded,
            ApiError::ServerError { status, message } => classify_context_window_error(
                Some(status),
                None,
                None,
                &message,
                Some("anthropic"),
                None,
            )
            .unwrap_or(LlmError::Server { status, message }),
            ApiError::SerializeError(msg) => LlmError::Serialize(msg),
        }
    }
}

// ---------------------------------------------------------------------------
// AnthropicProvider
// ---------------------------------------------------------------------------

/// Anthropic tier alias map — provider-owned model identifiers indexed by
/// agent class.
///
/// Defaults match `archon_core::config::AnthropicModelsConfig::default()`.
/// The binary should populate this from the operator's `[models.anthropic]`
/// config and pass it to `AnthropicProvider::with_alias_map(..)` so config
/// overrides flow through to provider resolution.
#[derive(Debug, Clone)]
pub struct AnthropicAliasMap {
    pub opus: String,
    pub sonnet: String,
    pub haiku: String,
}

impl Default for AnthropicAliasMap {
    fn default() -> Self {
        Self {
            opus: "claude-opus-4-7".into(),
            sonnet: "claude-sonnet-4-6".into(),
            haiku: "claude-haiku-4-5-20251001".into(),
        }
    }
}

/// An `LlmProvider` backed by `AnthropicClient`.
///
/// The inner client remains accessible via `client()` for code paths that
/// need Anthropic-specific accessors (auth headers, identity headers).
pub struct AnthropicProvider {
    client: AnthropicClient,
    aliases: AnthropicAliasMap,
}

impl AnthropicProvider {
    /// Wrap an existing `AnthropicClient` using compile-time-default aliases.
    ///
    /// Use `with_alias_map(..)` to supply an operator-overridden alias map
    /// from `ArchonConfig::models.anthropic`.
    pub fn new(client: AnthropicClient) -> Self {
        Self {
            client,
            aliases: AnthropicAliasMap::default(),
        }
    }

    /// Builder: attach an alias map sourced from operator config.
    pub fn with_alias_map(mut self, aliases: AnthropicAliasMap) -> Self {
        self.aliases = aliases;
        self
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
                id: "claude-opus-4-7".into(),
                display_name: "Claude Opus 4.7".into(),
                context_window: 0,
            },
            ModelInfo {
                id: "claude-sonnet-4-6".into(),
                display_name: "Claude Sonnet 4.6".into(),
                context_window: 0,
            },
            ModelInfo {
                id: "claude-haiku-4-5-20251001".into(),
                display_name: "Claude Haiku 4.5".into(),
                context_window: 0,
            },
        ]
    }

    fn resolve_alias(&self, alias: &str) -> Option<String> {
        match alias.trim().to_lowercase().as_str() {
            "opus" => Some(self.aliases.opus.clone()),
            "sonnet" => Some(self.aliases.sonnet.clone()),
            "haiku" => Some(self.aliases.haiku.clone()),
            _ => None,
        }
    }

    async fn stream(&self, mut request: LlmRequest) -> Result<Receiver<StreamEvent>, LlmError> {
        self.resolve_request_model(&mut request);
        request.messages = crate::message_invariants::sanitize_anthropic_shape(request.messages);
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

    fn data_flow_classification(&self) -> DataFlowClassification {
        classify_data_flow_endpoint(self.client.api_url())
    }
}
