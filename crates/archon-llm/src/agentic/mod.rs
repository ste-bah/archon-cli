//! Provider-neutral agentic turn contract.
//!
//! This is the PRD-FINALISATION-002 seam between high-level Archon agent loops
//! and provider-specific Anthropic/Codex wire formats. It intentionally wraps
//! the existing `LlmProvider` stream API first; later phases can add native
//! Codex tool-result continuation without changing session/subagent callers.

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::provider::{LlmError, LlmProvider, LlmRequest, ProviderFeature};
use crate::providers::ProviderCapability;
use crate::types::Usage;

mod adapter;

pub use adapter::LlmProviderAgenticAdapter;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone)]
pub struct AgenticTurnRequest {
    pub model: String,
    pub max_tokens: u32,
    pub system: Vec<serde_json::Value>,
    pub messages: Vec<serde_json::Value>,
    pub tools: Vec<serde_json::Value>,
    pub tool_results: Vec<AgenticToolResult>,
    pub effort: Option<String>,
    pub reasoning_encrypted: Option<String>,
    pub session_id: Option<String>,
    pub run_id: Option<String>,
    pub parent_id: Option<String>,
    pub budget_usd_remaining: Option<f64>,
    pub extra: serde_json::Value,
}

impl AgenticTurnRequest {
    pub fn into_llm_request(mut self) -> LlmRequest {
        append_tool_results(&mut self.messages, &self.tool_results);
        LlmRequest {
            model: self.model,
            max_tokens: self.max_tokens,
            system: self.system,
            messages: self.messages,
            tools: self.tools,
            thinking: None,
            speed: None,
            effort: self.effort,
            extra: self.extra,
            request_origin: Some("agentic_turn".into()),
            reasoning_encrypted: self.reasoning_encrypted,
        }
    }
}

impl From<LlmRequest> for AgenticTurnRequest {
    fn from(request: LlmRequest) -> Self {
        Self {
            model: request.model,
            max_tokens: request.max_tokens,
            system: request.system,
            messages: request.messages,
            tools: request.tools,
            tool_results: Vec::new(),
            effort: request.effort,
            reasoning_encrypted: request.reasoning_encrypted,
            session_id: None,
            run_id: None,
            parent_id: None,
            budget_usd_remaining: None,
            extra: request.extra,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgenticToolResult {
    pub tool_call_id: String,
    pub content: serde_json::Value,
    pub is_error: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgenticToolCall {
    pub id: String,
    pub name: String,
    pub arguments_raw: String,
    pub arguments: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub enum AgenticTurnEvent {
    MessageStarted { provider_id: String, model: String },
    TextDelta { text: String },
    ReasoningDelta { text: String },
    ToolCallStarted { id: String, name: String },
    ToolCallArgumentDelta { id: String, delta: String },
    ToolCallCompleted { call: AgenticToolCall },
    UsageUpdated { usage: Usage },
    ReasoningEncrypted { blob: String },
    ProviderWarning { message: String },
    ProviderError { message: String },
    TurnCompleted { stop_reason: Option<String> },
}

#[derive(Debug, Clone)]
pub struct AgenticTurnOutcome {
    pub provider_id: String,
    pub model: String,
    pub text: String,
    pub tool_calls: Vec<AgenticToolCall>,
    pub usage: Usage,
    pub stop_reason: Option<String>,
    pub reasoning_encrypted: Option<String>,
}

#[derive(Clone, Default)]
pub struct TurnEventSink {
    tx: Option<mpsc::Sender<AgenticTurnEvent>>,
}

impl TurnEventSink {
    pub fn channel(buffer: usize) -> (Self, mpsc::Receiver<AgenticTurnEvent>) {
        let (tx, rx) = mpsc::channel(buffer);
        (Self { tx: Some(tx) }, rx)
    }

    pub fn discard() -> Self {
        Self { tx: None }
    }

    pub async fn emit(&self, event: AgenticTurnEvent) -> Result<(), LlmError> {
        if let Some(tx) = &self.tx {
            tx.send(event).await.map_err(|_| LlmError::Aborted)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderCapabilitySet {
    capabilities: Vec<ProviderCapability>,
}

impl ProviderCapabilitySet {
    pub fn new(capabilities: impl IntoIterator<Item = ProviderCapability>) -> Self {
        Self {
            capabilities: capabilities.into_iter().collect(),
        }
    }

    pub fn supports(&self, capability: ProviderCapability) -> bool {
        self.capabilities.contains(&capability)
    }

    pub fn as_slice(&self) -> &[ProviderCapability] {
        &self.capabilities
    }

    pub fn from_llm_provider(provider: &dyn LlmProvider) -> Self {
        let mut capabilities = Vec::new();
        if provider.supports_feature(ProviderFeature::Streaming) {
            capabilities.push(ProviderCapability::Streaming);
        }
        if provider.supports_feature(ProviderFeature::ToolUse) {
            capabilities.push(ProviderCapability::ToolUse);
        }
        if provider.supports_feature(ProviderFeature::Vision) {
            capabilities.push(ProviderCapability::Vision);
        }
        Self::new(capabilities)
    }
}

#[async_trait]
pub trait AgenticLlmProvider: Send + Sync {
    async fn stream_turn(
        &self,
        request: AgenticTurnRequest,
        sink: TurnEventSink,
    ) -> Result<AgenticTurnOutcome, LlmError>;

    fn capabilities(&self) -> ProviderCapabilitySet;
    fn provider_id(&self) -> &str;
    fn model_id(&self) -> &str;
}

fn append_tool_results(messages: &mut Vec<serde_json::Value>, results: &[AgenticToolResult]) {
    if results.is_empty() {
        return;
    }
    let content = results
        .iter()
        .map(|result| {
            serde_json::json!({
                "type": "tool_result",
                "tool_use_id": result.tool_call_id,
                "content": result.content,
                "is_error": result.is_error,
            })
        })
        .collect::<Vec<_>>();
    messages.push(serde_json::json!({
        "role": "user",
        "content": content,
    }));
}
