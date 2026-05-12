//! LLM client adapters for the pipeline's [`LlmClient`] trait.

use std::sync::{Arc, Mutex};

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
    compact_state: Mutex<archon_core::agent::AutoCompactState>,
}

impl ProviderLlmAdapter {
    pub fn new(provider: Arc<dyn LlmProvider>) -> Self {
        Self {
            provider,
            max_tokens: 8192,
            request_origin: Some("pipeline".into()),
            compact_state: Mutex::new(archon_core::agent::AutoCompactState::default()),
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

    fn compact_for_request(
        &self,
        messages: Vec<serde_json::Value>,
        model: &str,
        force: bool,
    ) -> Result<Vec<serde_json::Value>> {
        let window =
            archon_llm::context_window::resolve_context_window(model, None, Some(&*self.provider))
                .context_window;
        let tokens = archon_core::agent::autocompact::estimate_messages_tokens(&messages);
        let action = if force {
            Some(archon_core::agent::CompactAction::Full)
        } else {
            let state = self
                .compact_state
                .lock()
                .map_err(|_| anyhow::anyhow!("pipeline compaction state lock poisoned"))?;
            archon_core::agent::evaluate_compaction(tokens, window, &state, 0.80)
        };
        let Some(action) = action else {
            return Ok(messages);
        };

        {
            let mut state = self
                .compact_state
                .lock()
                .map_err(|_| anyhow::anyhow!("pipeline compaction state lock poisoned"))?;
            if !force && !state.should_attempt() {
                return Ok(messages);
            }
            state.compact_in_flight = true;
        }

        match archon_core::agent::autocompact::compact_json_messages_apply(&messages, action) {
            Ok(compacted) => {
                let after = archon_core::agent::autocompact::estimate_messages_tokens(&compacted);
                self.compact_state
                    .lock()
                    .map_err(|_| anyhow::anyhow!("pipeline compaction state lock poisoned"))?
                    .on_success(after);
                tracing::info!(
                    before_tokens = tokens,
                    after_tokens = after,
                    force,
                    "pipeline prompt compacted"
                );
                Ok(compacted)
            }
            Err(err) => {
                self.compact_state
                    .lock()
                    .map_err(|_| anyhow::anyhow!("pipeline compaction state lock poisoned"))?
                    .on_real_failure();
                if force {
                    Err(anyhow::anyhow!(
                        "pipeline reactive compaction failed: {err}"
                    ))
                } else {
                    tracing::warn!(error = %err, "pipeline prompt compaction skipped");
                    Ok(messages)
                }
            }
        }
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
        let effective_model = self.model_for_provider(model);
        let compacted_messages = self.compact_for_request(messages, &effective_model, false)?;
        let request = LlmRequest {
            model: effective_model.clone(),
            max_tokens: self.max_tokens,
            system,
            messages: compacted_messages,
            tools,
            request_origin: self.request_origin.clone(),
            ..LlmRequest::default()
        };

        let rx = match self.provider.stream(request.clone()).await {
            Ok(rx) => rx,
            Err(e) if e.is_context_window_exceeded() => {
                let retry_messages =
                    self.compact_for_request(request.messages.clone(), &effective_model, true)?;
                self.provider
                    .stream(LlmRequest {
                        messages: retry_messages,
                        ..request
                    })
                    .await
                    .map_err(anyhow::Error::new)?
            }
            Err(e) => return Err(anyhow::anyhow!("LLM API error: {e}")),
        };

        collect_stream(rx).await
    }
}

async fn collect_stream(mut rx: Receiver<StreamEvent>) -> Result<LlmResponse> {
    let mut text_parts: Vec<String> = Vec::new();
    let mut tool_uses: Vec<ToolUseEntry> = Vec::new();
    let mut usage = archon_llm::usage::UsageAccumulator::default();

    // Track in-progress tool_use blocks by content-block index.
    let mut active_tool_blocks: std::collections::HashMap<u32, (String, String, String)> =
        std::collections::HashMap::new();

    while let Some(event) = rx.recv().await {
        usage.record_event(&event);
        match event {
            StreamEvent::MessageStart { .. } => {}
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
            StreamEvent::MessageDelta { .. } => {}
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
                if let Some(err) = archon_llm::context_window::classify_context_window_error(
                    None,
                    Some(&error_type),
                    None,
                    &message,
                    Some("pipeline"),
                    None,
                ) {
                    return Err(anyhow::Error::new(err));
                }
                anyhow::bail!(
                    "LLM stream error ({error_type}): {message}; partial_output_hash={partial_hash}"
                );
            }
        }
    }

    Ok(LlmResponse {
        content: text_parts.join(""),
        tool_uses,
        tokens_in: usage.context_input_tokens,
        tokens_out: usage.output_tokens,
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

    struct FakeProvider {
        name: &'static str,
        model: &'static str,
        context_window: u32,
        seen_model: std::sync::Mutex<Option<String>>,
        seen_messages: std::sync::Mutex<Vec<serde_json::Value>>,
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
                context_window: self.context_window,
            }]
        }

        async fn stream(&self, request: LlmRequest) -> Result<Receiver<StreamEvent>, LlmError> {
            *self
                .seen_model
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(request.model);
            *self
                .seen_messages
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = request.messages;
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
            context_window: 200_000,
            seen_model: std::sync::Mutex::new(None),
            seen_messages: std::sync::Mutex::new(Vec::new()),
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
            context_window: 200_000,
            seen_model: std::sync::Mutex::new(None),
            seen_messages: std::sync::Mutex::new(Vec::new()),
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

    #[tokio::test]
    async fn provider_adapter_compacts_prompt_with_shared_lifecycle() {
        let provider = Arc::new(FakeProvider {
            name: "openai-codex",
            model: "gpt-5.4",
            context_window: 128,
            seen_model: std::sync::Mutex::new(None),
            seen_messages: std::sync::Mutex::new(Vec::new()),
        });
        let seen = Arc::clone(&provider);
        let adapter = ProviderLlmAdapter::new(provider);
        let messages: Vec<_> = (0..10)
            .map(|i| serde_json::json!({"role": "user", "content": "x".repeat(100 + i)}))
            .collect();

        let _ = adapter
            .send_message(messages, Vec::new(), Vec::new(), "gpt-5.4")
            .await
            .expect("fake provider response");

        let sent = seen
            .seen_messages
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        assert!(sent.len() < 10);
        assert!(
            sent.first()
                .and_then(|msg| msg.get("content"))
                .and_then(|content| content.as_str())
                .unwrap_or_default()
                .contains("Context Summary")
        );
    }
}
