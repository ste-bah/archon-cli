//! LLM client adapters for the pipeline's [`LlmClient`] trait.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;
use async_trait::async_trait;
use sha2::{Digest, Sha256};

use archon_core::agent::{AgentConfig, RuntimeAttribution};
use archon_llm::anthropic::{AnthropicClient, MessageRequest};
use archon_llm::provider::{LlmProvider, LlmRequest};
use archon_llm::streaming::StreamEvent;
use tokio::sync::mpsc::Receiver;

use crate::kb::compile::KbLlmClient;
use crate::kb::query::{AnswerStreamSink, QaSynthesizer};
use crate::runner::{AgentExecutionRequest, LlmClient, LlmResponse, ToolUseEntry};

/// Callback handed each text delta as it leaves the provider stream.
///
/// `Send` because it is held across the `await` inside the stream loop.
type TextDeltaSink<'a> = &'a mut (dyn FnMut(&str) -> Result<()> + Send);

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
    run_id: String,
    session_id: String,
    next_round: AtomicU64,
}

impl ProviderLlmAdapter {
    pub fn new(provider: Arc<dyn LlmProvider>) -> Self {
        Self {
            provider,
            max_tokens: 8192,
            request_origin: Some("pipeline".into()),
            run_id: uuid::Uuid::new_v4().to_string(),
            session_id: uuid::Uuid::new_v4().to_string(),
            next_round: AtomicU64::new(0),
        }
    }

    pub fn with_origin(mut self, origin: impl Into<String>) -> Self {
        self.request_origin = Some(origin.into());
        self
    }

    fn runtime_extra(&self, run_id: &str, session_id: &str) -> serde_json::Value {
        AgentConfig::default().runtime_attribution_extra_for_scope(RuntimeAttribution {
            run_id,
            session_id,
            role: "pipeline",
            origin: self.request_origin.as_deref().unwrap_or("pipeline"),
            turn: None,
            round: Some(self.next_round.fetch_add(1, Ordering::Relaxed)),
            denominator: None,
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn send_with_scope(
        &self,
        messages: Vec<serde_json::Value>,
        system: Vec<serde_json::Value>,
        tools: Vec<serde_json::Value>,
        model: &str,
        run_id: &str,
        session_id: &str,
        on_text: Option<TextDeltaSink<'_>>,
    ) -> Result<LlmResponse> {
        let effective_model = self.model_for_provider(model);
        let request = LlmRequest {
            model: effective_model.clone(),
            max_tokens: self.max_tokens,
            system,
            messages,
            tools,
            request_origin: self.request_origin.clone(),
            extra: self.runtime_extra(run_id, session_id),
            ..LlmRequest::default()
        };

        let rx = match self.provider.stream(request.clone()).await {
            Ok(rx) => rx,
            Err(e) if e.is_context_window_exceeded() => return Err(anyhow::Error::new(e)),
            Err(e) => return Err(anyhow::anyhow!("LLM API error: {e}")),
        };

        collect_stream_into(rx, on_text).await
    }

    fn model_for_provider(&self, requested: &str) -> String {
        let mut request = LlmRequest {
            model: requested.to_string(),
            ..LlmRequest::default()
        };
        self.provider.resolve_request_model(&mut request);
        if request.model != requested {
            return request.model;
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
    fn provider_id(&self) -> Option<String> {
        Some(self.provider.name().to_string())
    }

    fn resolve_model_alias(&self, model: &str) -> String {
        self.model_for_provider(model)
    }

    async fn send_message(
        &self,
        messages: Vec<serde_json::Value>,
        system: Vec<serde_json::Value>,
        tools: Vec<serde_json::Value>,
        model: &str,
    ) -> Result<LlmResponse> {
        self.send_with_scope(
            messages,
            system,
            tools,
            model,
            &self.run_id,
            &self.session_id,
            None,
        )
        .await
    }

    async fn run_agent(&self, request: AgentExecutionRequest) -> Result<LlmResponse> {
        if let Some(cwd) = request.cwd.as_ref() {
            anyhow::bail!(
                "ProviderLlmAdapter cannot execute cwd-bound agent request for '{}'; wrap it in SubagentPipelineClient so cwd '{}' becomes the subagent workspace root",
                request.agent.key,
                cwd.display()
            );
        }
        let model = request.agent.model.clone();
        self.send_with_scope(
            request.messages,
            request.system,
            request.tools,
            &model,
            &request.session_id,
            &request.session_id,
            None,
        )
        .await
    }
}

// ---------------------------------------------------------------------------
// Knowledge-base adapters
// ---------------------------------------------------------------------------

/// Single-prompt completion over a resolved provider, for the knowledge-base
/// passes.
///
/// `kb::compile` and `kb::query` take narrow abstract traits — a prompt in,
/// text out — so their tests can run deterministically without a model. This is
/// the one production implementation of both, and it lives here rather than in
/// `archon-pipeline::kb` for the same reason [`ProviderLlmAdapter`] does: the
/// knowledge-base modules should not know which provider is configured.
pub struct KbProviderClient {
    inner: ProviderLlmAdapter,
    model: String,
}

impl KbProviderClient {
    /// Wrap a resolved provider. `model` is an alias or concrete ID; the inner
    /// adapter resolves it the same way every other pipeline call does.
    pub fn new(provider: Arc<dyn LlmProvider>, model: impl Into<String>) -> Self {
        Self {
            inner: ProviderLlmAdapter::new(provider).with_origin("kb"),
            model: model.into(),
        }
    }

    /// One prompt in, complete text out, with every delta handed to `on_text`
    /// first when one is supplied.
    async fn complete_text(
        &self,
        prompt: &str,
        on_text: Option<TextDeltaSink<'_>>,
    ) -> Result<String> {
        let messages = vec![serde_json::json!({
            "role": "user",
            "content": [{ "type": "text", "text": prompt }],
        })];
        let response = self
            .inner
            .send_with_scope(
                messages,
                Vec::new(),
                Vec::new(),
                &self.model,
                &self.inner.run_id,
                &self.inner.session_id,
                on_text,
            )
            .await?;
        Ok(response.content)
    }
}

#[async_trait]
impl KbLlmClient for KbProviderClient {
    async fn complete(&self, prompt: &str) -> Result<String> {
        self.complete_text(prompt, None).await
    }
}

#[async_trait]
impl QaSynthesizer for KbProviderClient {
    /// The engine has already assembled the instruction and the evidence into
    /// `context`; the bare question is kept only for logging by other
    /// implementations, so it is not re-sent here.
    async fn synthesize(&self, _question: &str, context: &str) -> Result<String> {
        self.complete_text(context, None).await
    }

    /// Streaming costs one callback here, not a second transport: every
    /// provider already delivers `StreamEvent`s and the non-streaming path
    /// differs only in that it throws the deltas away until the stream ends.
    async fn synthesize_streaming(
        &self,
        _question: &str,
        context: &str,
        sink: &mut dyn AnswerStreamSink,
    ) -> Result<String> {
        let mut forward = |text: &str| sink.on_token(text);
        self.complete_text(context, Some(&mut forward)).await
    }
}

async fn collect_stream(rx: Receiver<StreamEvent>) -> Result<LlmResponse> {
    collect_stream_into(rx, None).await
}

/// Drain a provider stream into a complete [`LlmResponse`], optionally handing
/// each text delta to `on_text` on the way past.
///
/// One loop understands `StreamEvent` for the whole pipeline. Streaming callers
/// get their tokens from the callback rather than from a parallel consumer,
/// because a `Receiver` has a single consumer and a second one would have to
/// re-implement error classification and usage accounting to match.
async fn collect_stream_into(
    mut rx: Receiver<StreamEvent>,
    mut on_text: Option<TextDeltaSink<'_>>,
) -> Result<LlmResponse> {
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
                if let Some(on_text) = on_text.as_deref_mut() {
                    on_text(&text)?;
                }
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

#[cfg(test)]
#[path = "llm_adapter_tests.rs"]
mod tests;
