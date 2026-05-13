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
        request.messages = sanitize_anthropic_messages(request.messages);
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

fn sanitize_anthropic_messages(messages: Vec<serde_json::Value>) -> Vec<serde_json::Value> {
    let mut sanitized = Vec::with_capacity(messages.len());
    for mut message in messages {
        normalize_message_role(&mut message);
        if has_tool_result(&message)
            && !previous_assistant_has_tool_uses(sanitized.last(), &message)
        {
            message = orphan_tool_result_as_text(&message);
        }
        sanitized.push(message);
    }
    sanitized
}

fn normalize_message_role(message: &mut serde_json::Value) {
    let role = message
        .get("role")
        .and_then(|v| v.as_str())
        .unwrap_or("user");
    if !matches!(role, "user" | "assistant") {
        message["role"] = serde_json::Value::String("user".into());
    }
}

fn has_tool_result(message: &serde_json::Value) -> bool {
    message
        .get("content")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .any(|block| block.get("type").and_then(|v| v.as_str()) == Some("tool_result"))
}

fn previous_assistant_has_tool_uses(
    previous: Option<&serde_json::Value>,
    result_message: &serde_json::Value,
) -> bool {
    let Some(previous) = previous else {
        return false;
    };
    if previous.get("role").and_then(|v| v.as_str()) != Some("assistant") {
        return false;
    }
    let result_ids = tool_result_ids(result_message);
    !result_ids.is_empty()
        && result_ids
            .iter()
            .all(|id| assistant_has_tool_use(previous, id))
}

fn tool_result_ids(message: &serde_json::Value) -> Vec<&str> {
    message
        .get("content")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter(|block| block.get("type").and_then(|v| v.as_str()) == Some("tool_result"))
        .filter_map(|block| block.get("tool_use_id").and_then(|v| v.as_str()))
        .collect()
}

fn assistant_has_tool_use(message: &serde_json::Value, id: &str) -> bool {
    message
        .get("content")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .any(|block| {
            block.get("type").and_then(|v| v.as_str()) == Some("tool_use")
                && block.get("id").and_then(|v| v.as_str()) == Some(id)
        })
}

fn orphan_tool_result_as_text(message: &serde_json::Value) -> serde_json::Value {
    let text = message
        .get("content")
        .and_then(|v| v.as_array())
        .map(|blocks| {
            blocks
                .iter()
                .map(tool_result_block_text)
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_else(|| {
            message
                .get("content")
                .cloned()
                .unwrap_or_default()
                .to_string()
        });
    serde_json::json!({ "role": "user", "content": text })
}

fn tool_result_block_text(block: &serde_json::Value) -> String {
    if block.get("type").and_then(|v| v.as_str()) == Some("tool_result") {
        let id = block
            .get("tool_use_id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let content = block
            .get("content")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| {
                block
                    .get("content")
                    .cloned()
                    .unwrap_or_default()
                    .to_string()
            });
        return format!("[Tool result {id}] {content}");
    }
    block
        .get("text")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| block.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizer_converts_system_messages_to_user() {
        let messages = sanitize_anthropic_messages(vec![serde_json::json!({
            "role": "system",
            "content": "boundary"
        })]);

        assert_eq!(messages[0]["role"], "user");
    }

    #[test]
    fn sanitizer_textifies_orphan_tool_result() {
        let messages = sanitize_anthropic_messages(vec![serde_json::json!({
            "role": "user",
            "content": [{
                "type": "tool_result",
                "tool_use_id": "tool-1",
                "content": "failed"
            }]
        })]);

        assert_eq!(messages[0]["role"], "user");
        assert!(messages[0]["content"].as_str().unwrap().contains("tool-1"));
    }

    #[test]
    fn sanitizer_preserves_valid_tool_pair() {
        let messages = sanitize_anthropic_messages(vec![
            serde_json::json!({
                "role": "assistant",
                "content": [{"type": "tool_use", "id": "tool-1", "name": "Read", "input": {}}]
            }),
            serde_json::json!({
                "role": "user",
                "content": [{"type": "tool_result", "tool_use_id": "tool-1", "content": "ok"}]
            }),
        ]);

        assert_eq!(messages[1]["content"][0]["type"], "tool_result");
    }
}
