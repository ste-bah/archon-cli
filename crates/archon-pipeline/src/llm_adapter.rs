//! LLM client adapters for the pipeline's [`LlmClient`] trait.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use sha2::{Digest, Sha256};

use archon_llm::anthropic::{AnthropicClient, MessageRequest};
use archon_llm::provider::{LlmProvider, LlmRequest};
use archon_llm::streaming::StreamEvent;
use tokio::sync::mpsc::Receiver;

use crate::runner::{LlmClient, LlmResponse, ToolUseEntry};

// ---------------------------------------------------------------------------
// Adapter
// ---------------------------------------------------------------------------

/// Wraps an [`AnthropicClient`] to implement the pipeline's [`LlmClient`] trait.
///
/// Converts the streaming `Receiver<StreamEvent>` API into a collected
/// [`LlmResponse`] suitable for the synchronous agent-loop in `run_pipeline`.
pub struct AnthropicLlmAdapter {
    client: Arc<AnthropicClient>,
}

impl AnthropicLlmAdapter {
    pub fn new(client: Arc<AnthropicClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl LlmClient for AnthropicLlmAdapter {
    async fn send_message(
        &self,
        messages: Vec<serde_json::Value>,
        system: Vec<serde_json::Value>,
        tools: Vec<serde_json::Value>,
        model: &str,
    ) -> Result<LlmResponse> {
        let request = MessageRequest {
            model: model.to_string(),
            max_tokens: 8192,
            system,
            messages,
            tools,
            thinking: None,
            speed: None,
            effort: None,
            request_origin: None,
        };

        let rx = self
            .client
            .stream_message(request)
            .await
            .map_err(|e| anyhow::anyhow!("LLM API error: {e}"))?;

        collect_stream(rx).await
    }
}

/// Provider-neutral adapter for pipelines.
///
/// This is the production path for Anthropic, Codex, and compatible providers
/// once a command has resolved the active [`LlmProvider`]. It keeps pipeline
/// facades provider-agnostic and prevents them from constructing Anthropic
/// clients directly.
pub struct ProviderLlmAdapter {
    provider: Arc<dyn LlmProvider>,
    max_tokens: u32,
    request_origin: Option<String>,
}

impl ProviderLlmAdapter {
    pub fn new(provider: Arc<dyn LlmProvider>) -> Self {
        Self {
            provider,
            max_tokens: 8192,
            request_origin: Some("pipeline".into()),
        }
    }

    pub fn with_origin(mut self, origin: impl Into<String>) -> Self {
        self.request_origin = Some(origin.into());
        self
    }

    fn model_for_provider(&self, requested: &str) -> String {
        // Tier aliases (`"sonnet"`, `"opus"`, `"haiku"`) get resolved by the
        // active provider via `LlmProvider::resolve_alias(..)`. Anthropic maps
        // them to its `claude-*` namespace; Codex maps to `gpt-*`; local /
        // OpenAI-compat returns `None` for pass-through.
        if let Some(resolved) = self.provider.resolve_alias(requested) {
            return resolved;
        }

        // Legacy compatibility: an explicit `claude-*` literal coming through
        // a non-Anthropic provider falls back to the provider's first model.
        // This matches pre-resolver behavior for agent code that still emits
        // concrete IDs instead of aliases.
        if requested.starts_with("claude") {
            return self
                .provider
                .models()
                .first()
                .map(|model| model.id.clone())
                .filter(|model| !model.starts_with("claude"))
                .unwrap_or_else(|| requested.to_string());
        }

        // Everything else (concrete IDs the provider recognises directly)
        // passes through.
        requested.to_string()
    }
}

#[async_trait]
impl LlmClient for ProviderLlmAdapter {
    async fn send_message(
        &self,
        messages: Vec<serde_json::Value>,
        system: Vec<serde_json::Value>,
        tools: Vec<serde_json::Value>,
        model: &str,
    ) -> Result<LlmResponse> {
        let request = LlmRequest {
            model: self.model_for_provider(model),
            max_tokens: self.max_tokens,
            system,
            messages,
            tools,
            request_origin: self.request_origin.clone(),
            ..LlmRequest::default()
        };

        let rx = self
            .provider
            .stream(request)
            .await
            .map_err(|e| anyhow::anyhow!("LLM API error: {e}"))?;

        collect_stream(rx).await
    }
}

async fn collect_stream(mut rx: Receiver<StreamEvent>) -> Result<LlmResponse> {
    let mut text_parts: Vec<String> = Vec::new();
    let mut tool_uses: Vec<ToolUseEntry> = Vec::new();
    let mut tokens_in: u64 = 0;
    let mut tokens_out: u64 = 0;

    // Track in-progress tool_use blocks by content-block index.
    let mut active_tool_blocks: std::collections::HashMap<u32, (String, String, String)> =
        std::collections::HashMap::new();

    while let Some(event) = rx.recv().await {
        match event {
            StreamEvent::MessageStart { usage, .. } => {
                tokens_in += usage.input_tokens;
                tokens_out += usage.output_tokens;
            }
            StreamEvent::ContentBlockStart {
                index,
                block_type,
                tool_use_id,
                tool_name,
            } => {
                if block_type == archon_llm::types::ContentBlockType::ToolUse {
                    active_tool_blocks.insert(
                        index,
                        (
                            tool_use_id.unwrap_or_default(),
                            tool_name.unwrap_or_default(),
                            String::new(),
                        ),
                    );
                }
            }
            StreamEvent::TextDelta { text, .. } => {
                text_parts.push(text);
            }
            StreamEvent::InputJsonDelta {
                index,
                partial_json,
            } => {
                if let Some(entry) = active_tool_blocks.get_mut(&index) {
                    entry.2.push_str(&partial_json);
                }
            }
            StreamEvent::ContentBlockStop { index } => {
                if let Some((_id, name, json_str)) = active_tool_blocks.remove(&index) {
                    let input: serde_json::Value =
                        serde_json::from_str(&json_str).unwrap_or(serde_json::Value::Null);
                    tool_uses.push(ToolUseEntry {
                        tool_name: name,
                        input,
                        output: serde_json::Value::Null,
                    });
                }
            }
            StreamEvent::MessageDelta { usage, .. } => {
                if let Some(u) = usage {
                    tokens_in += u.input_tokens;
                    tokens_out += u.output_tokens;
                }
            }
            StreamEvent::ThinkingDelta { .. }
            | StreamEvent::SignatureDelta { .. }
            | StreamEvent::ReasoningEncrypted { .. }
            | StreamEvent::MessageStop
            | StreamEvent::Ping => {}
            StreamEvent::Error {
                error_type,
                message,
            } => {
                let partial_hash = if text_parts.is_empty() {
                    "none".to_string()
                } else {
                    let partial = text_parts.join("");
                    let digest = Sha256::digest(partial.as_bytes());
                    hex::encode(digest)
                };
                anyhow::bail!(
                    "LLM stream error ({error_type}): {message}; partial_output_hash={partial_hash}"
                );
            }
        }
    }

    Ok(LlmResponse {
        content: text_parts.join(""),
        tool_uses,
        tokens_in,
        tokens_out,
    })
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use archon_llm::provider::{LlmError, ModelInfo, ProviderFeature};
    use archon_llm::types::Usage;

    /// Helper: create an LlmResponse from a vec of StreamEvents by simulating
    /// the collection logic (without requiring an actual AnthropicClient).
    fn collect_events(events: Vec<StreamEvent>) -> LlmResponse {
        let mut text_parts: Vec<String> = Vec::new();
        let mut tool_uses: Vec<ToolUseEntry> = Vec::new();
        let mut tokens_in: u64 = 0;
        let mut tokens_out: u64 = 0;
        let mut active_tool_blocks: std::collections::HashMap<u32, (String, String, String)> =
            std::collections::HashMap::new();

        for event in events {
            match event {
                StreamEvent::MessageStart { usage, .. } => {
                    tokens_in += usage.input_tokens;
                    tokens_out += usage.output_tokens;
                }
                StreamEvent::ContentBlockStart {
                    index,
                    block_type,
                    tool_use_id,
                    tool_name,
                } => {
                    if block_type == archon_llm::types::ContentBlockType::ToolUse {
                        active_tool_blocks.insert(
                            index,
                            (
                                tool_use_id.unwrap_or_default(),
                                tool_name.unwrap_or_default(),
                                String::new(),
                            ),
                        );
                    }
                }
                StreamEvent::TextDelta { text, .. } => {
                    text_parts.push(text);
                }
                StreamEvent::InputJsonDelta {
                    index,
                    partial_json,
                } => {
                    if let Some(entry) = active_tool_blocks.get_mut(&index) {
                        entry.2.push_str(&partial_json);
                    }
                }
                StreamEvent::ContentBlockStop { index } => {
                    if let Some((_id, name, json_str)) = active_tool_blocks.remove(&index) {
                        let input: serde_json::Value =
                            serde_json::from_str(&json_str).unwrap_or(serde_json::Value::Null);
                        tool_uses.push(ToolUseEntry {
                            tool_name: name,
                            input,
                            output: serde_json::Value::Null,
                        });
                    }
                }
                StreamEvent::MessageDelta { usage, .. } => {
                    if let Some(u) = usage {
                        tokens_in += u.input_tokens;
                        tokens_out += u.output_tokens;
                    }
                }
                _ => {}
            }
        }

        LlmResponse {
            content: text_parts.join(""),
            tool_uses,
            tokens_in,
            tokens_out,
        }
    }

    #[test]
    fn empty_stream_produces_empty_response() {
        let response = collect_events(vec![]);
        assert!(response.content.is_empty());
        assert!(response.tool_uses.is_empty());
        assert_eq!(response.tokens_in, 0);
        assert_eq!(response.tokens_out, 0);
    }

    #[test]
    fn text_content_blocks_concatenated() {
        let events = vec![
            StreamEvent::MessageStart {
                id: "msg_1".into(),
                model: "claude-sonnet-4-6".into(),
                usage: archon_llm::types::Usage {
                    input_tokens: 100,
                    output_tokens: 0,
                    ..Default::default()
                },
            },
            StreamEvent::ContentBlockStart {
                index: 0,
                block_type: archon_llm::types::ContentBlockType::Text,
                tool_use_id: None,
                tool_name: None,
            },
            StreamEvent::TextDelta {
                index: 0,
                text: "Hello ".into(),
            },
            StreamEvent::TextDelta {
                index: 0,
                text: "world".into(),
            },
            StreamEvent::ContentBlockStop { index: 0 },
            StreamEvent::MessageDelta {
                stop_reason: Some("end_turn".into()),
                usage: Some(archon_llm::types::Usage {
                    input_tokens: 0,
                    output_tokens: 20,
                    ..Default::default()
                }),
            },
            StreamEvent::MessageStop,
        ];

        let response = collect_events(events);
        assert_eq!(response.content, "Hello world");
        assert!(response.tool_uses.is_empty());
        assert_eq!(response.tokens_in, 100);
        assert_eq!(response.tokens_out, 20);
    }

    #[test]
    fn tool_use_blocks_collected() {
        let events = vec![
            StreamEvent::MessageStart {
                id: "msg_2".into(),
                model: "claude-sonnet-4-6".into(),
                usage: archon_llm::types::Usage {
                    input_tokens: 50,
                    output_tokens: 0,
                    ..Default::default()
                },
            },
            StreamEvent::ContentBlockStart {
                index: 0,
                block_type: archon_llm::types::ContentBlockType::ToolUse,
                tool_use_id: Some("toolu_1".into()),
                tool_name: Some("read_file".into()),
            },
            StreamEvent::InputJsonDelta {
                index: 0,
                partial_json: r#"{"path":""#.into(),
            },
            StreamEvent::InputJsonDelta {
                index: 0,
                partial_json: r#"src/main.rs"}"#.into(),
            },
            StreamEvent::ContentBlockStop { index: 0 },
            StreamEvent::MessageStop,
        ];

        let response = collect_events(events);
        assert!(response.content.is_empty());
        assert_eq!(response.tool_uses.len(), 1);
        assert_eq!(response.tool_uses[0].tool_name, "read_file");
        assert_eq!(response.tool_uses[0].input["path"], "src/main.rs");
        assert_eq!(response.tokens_in, 50);
    }

    struct FakeProvider {
        name: &'static str,
        model: &'static str,
        seen_model: std::sync::Mutex<Option<String>>,
    }

    #[async_trait]
    impl LlmProvider for FakeProvider {
        fn name(&self) -> &str {
            self.name
        }

        fn models(&self) -> Vec<ModelInfo> {
            vec![ModelInfo {
                id: self.model.into(),
                display_name: self.model.into(),
                context_window: 200_000,
            }]
        }

        async fn stream(&self, request: LlmRequest) -> Result<Receiver<StreamEvent>, LlmError> {
            *self
                .seen_model
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(request.model);
            let (tx, rx) = tokio::sync::mpsc::channel(8);
            tokio::spawn(async move {
                let _ = tx
                    .send(StreamEvent::MessageStart {
                        id: "msg_fake".into(),
                        model: "gpt-5.4".into(),
                        usage: Usage {
                            input_tokens: 7,
                            ..Default::default()
                        },
                    })
                    .await;
                let _ = tx
                    .send(StreamEvent::TextDelta {
                        index: 0,
                        text: "pipeline-ok".into(),
                    })
                    .await;
                let _ = tx.send(StreamEvent::MessageStop).await;
            });
            Ok(rx)
        }

        async fn complete(
            &self,
            _request: LlmRequest,
        ) -> Result<archon_llm::provider::LlmResponse, LlmError> {
            Err(LlmError::Unsupported("fake provider complete".into()))
        }

        fn supports_feature(&self, feature: ProviderFeature) -> bool {
            matches!(feature, ProviderFeature::Streaming)
        }
    }

    #[tokio::test]
    async fn provider_adapter_collects_text_from_generic_provider() {
        let provider = Arc::new(FakeProvider {
            name: "openai-codex",
            model: "gpt-5.4",
            seen_model: std::sync::Mutex::new(None),
        });
        let adapter = ProviderLlmAdapter::new(provider);

        let response = adapter
            .send_message(Vec::new(), Vec::new(), Vec::new(), "gpt-5.4")
            .await
            .expect("fake provider response");

        assert_eq!(response.content, "pipeline-ok");
        assert_eq!(response.tokens_in, 7);
    }

    #[tokio::test]
    async fn provider_adapter_remaps_claude_agent_model_to_provider_default() {
        let provider = Arc::new(FakeProvider {
            name: "openai-codex",
            model: "gpt-5.4",
            seen_model: std::sync::Mutex::new(None),
        });
        let seen = Arc::clone(&provider);
        let adapter = ProviderLlmAdapter::new(provider);

        let _ = adapter
            .send_message(Vec::new(), Vec::new(), Vec::new(), "claude-sonnet-4-6")
            .await
            .expect("fake provider response");

        let model = seen
            .seen_model
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        assert_eq!(model.as_deref(), Some("gpt-5.4"));
    }
}
