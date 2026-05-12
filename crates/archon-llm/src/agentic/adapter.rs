use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;

use super::{
    AgenticLlmProvider, AgenticToolCall, AgenticTurnEvent, AgenticTurnOutcome, AgenticTurnRequest,
    ProviderCapabilitySet, TurnEventSink,
};
use crate::provider::{LlmError, LlmProvider};
use crate::streaming::StreamEvent;
use crate::types::{ContentBlockType, Usage};

pub struct LlmProviderAgenticAdapter {
    provider: Arc<dyn LlmProvider>,
    provider_id: String,
    model_id: String,
    capabilities: ProviderCapabilitySet,
}

impl LlmProviderAgenticAdapter {
    pub fn new(
        provider: Arc<dyn LlmProvider>,
        provider_id: impl Into<String>,
        model_id: impl Into<String>,
        capabilities: ProviderCapabilitySet,
    ) -> Self {
        Self {
            provider,
            provider_id: provider_id.into(),
            model_id: model_id.into(),
            capabilities,
        }
    }
}

#[async_trait]
impl AgenticLlmProvider for LlmProviderAgenticAdapter {
    async fn stream_turn(
        &self,
        request: AgenticTurnRequest,
        sink: TurnEventSink,
    ) -> Result<AgenticTurnOutcome, LlmError> {
        let mut rx = self.provider.stream(request.into_llm_request()).await?;
        let mut state = AgenticTurnState::new(self.provider_id.clone(), self.model_id.clone());

        while let Some(event) = rx.recv().await {
            state.process_event(event, &sink).await?;
        }

        Ok(state.into_outcome())
    }

    fn capabilities(&self) -> ProviderCapabilitySet {
        self.capabilities.clone()
    }

    fn provider_id(&self) -> &str {
        &self.provider_id
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }
}

struct ActiveToolCall {
    id: String,
    name: String,
    arguments_raw: String,
}

struct AgenticTurnState {
    provider_id: String,
    model: String,
    text: String,
    usage: Usage,
    stop_reason: Option<String>,
    reasoning_encrypted: Option<String>,
    active_tools: HashMap<u32, ActiveToolCall>,
    tool_calls: Vec<AgenticToolCall>,
}

impl AgenticTurnState {
    fn new(provider_id: String, model: String) -> Self {
        Self {
            provider_id,
            model,
            text: String::new(),
            usage: Usage::default(),
            stop_reason: None,
            reasoning_encrypted: None,
            active_tools: HashMap::new(),
            tool_calls: Vec::new(),
        }
    }

    async fn process_event(
        &mut self,
        event: StreamEvent,
        sink: &TurnEventSink,
    ) -> Result<(), LlmError> {
        match event {
            StreamEvent::MessageStart { model, usage, .. } => {
                self.message_start(model, usage, sink).await?;
            }
            StreamEvent::ContentBlockStart {
                index,
                block_type: ContentBlockType::ToolUse,
                tool_use_id,
                tool_name,
            } => {
                self.tool_start(index, tool_use_id, tool_name, sink).await?;
            }
            StreamEvent::TextDelta { text, .. } => {
                self.text.push_str(&text);
                sink.emit(AgenticTurnEvent::TextDelta { text }).await?;
            }
            StreamEvent::ThinkingDelta { thinking, .. } => {
                sink.emit(AgenticTurnEvent::ReasoningDelta { text: thinking })
                    .await?;
            }
            StreamEvent::InputJsonDelta {
                index,
                partial_json,
            } => self.tool_args_delta(index, partial_json, sink).await?,
            StreamEvent::ContentBlockStop { index } => self.content_stop(index, sink).await?,
            StreamEvent::MessageDelta { stop_reason, usage } => {
                self.message_delta(stop_reason, usage, sink).await?;
            }
            StreamEvent::ReasoningEncrypted { blob } => {
                self.reasoning_encrypted = Some(blob.clone());
                sink.emit(AgenticTurnEvent::ReasoningEncrypted { blob })
                    .await?;
            }
            StreamEvent::Error { message, .. } => {
                sink.emit(AgenticTurnEvent::ProviderError {
                    message: message.clone(),
                })
                .await?;
                return Err(LlmError::Http(message));
            }
            StreamEvent::MessageStop => {
                sink.emit(AgenticTurnEvent::TurnCompleted {
                    stop_reason: self.stop_reason.clone(),
                })
                .await?;
            }
            StreamEvent::ContentBlockStart { .. }
            | StreamEvent::SignatureDelta { .. }
            | StreamEvent::Ping => {}
        }
        Ok(())
    }

    async fn message_start(
        &mut self,
        model: String,
        usage: Usage,
        sink: &TurnEventSink,
    ) -> Result<(), LlmError> {
        if !model.is_empty() {
            self.model = model;
        }
        self.usage.merge(&usage);
        sink.emit(AgenticTurnEvent::MessageStarted {
            provider_id: self.provider_id.clone(),
            model: self.model.clone(),
        })
        .await
    }

    async fn tool_start(
        &mut self,
        index: u32,
        tool_use_id: Option<String>,
        tool_name: Option<String>,
        sink: &TurnEventSink,
    ) -> Result<(), LlmError> {
        let id = tool_use_id.unwrap_or_else(|| format!("tool-{index}"));
        let name = tool_name.unwrap_or_default();
        self.active_tools.insert(
            index,
            ActiveToolCall {
                id: id.clone(),
                name: name.clone(),
                arguments_raw: String::new(),
            },
        );
        sink.emit(AgenticTurnEvent::ToolCallStarted { id, name })
            .await
    }

    async fn tool_args_delta(
        &mut self,
        index: u32,
        partial_json: String,
        sink: &TurnEventSink,
    ) -> Result<(), LlmError> {
        if let Some(tool) = self.active_tools.get_mut(&index) {
            tool.arguments_raw.push_str(&partial_json);
            sink.emit(AgenticTurnEvent::ToolCallArgumentDelta {
                id: tool.id.clone(),
                delta: partial_json,
            })
            .await?;
        }
        Ok(())
    }

    async fn content_stop(&mut self, index: u32, sink: &TurnEventSink) -> Result<(), LlmError> {
        if let Some(tool) = self.active_tools.remove(&index) {
            let arguments = parse_tool_arguments(&tool, sink).await?;
            let call = AgenticToolCall {
                id: tool.id,
                name: tool.name,
                arguments,
                arguments_raw: tool.arguments_raw,
            };
            self.tool_calls.push(call.clone());
            sink.emit(AgenticTurnEvent::ToolCallCompleted { call })
                .await?;
        }
        Ok(())
    }

    async fn message_delta(
        &mut self,
        stop_reason: Option<String>,
        usage: Option<Usage>,
        sink: &TurnEventSink,
    ) -> Result<(), LlmError> {
        if let Some(usage) = usage {
            self.usage.merge(&usage);
            sink.emit(AgenticTurnEvent::UsageUpdated {
                usage: self.usage.clone(),
            })
            .await?;
        }
        if stop_reason.is_some() {
            self.stop_reason = stop_reason;
        }
        Ok(())
    }

    fn into_outcome(self) -> AgenticTurnOutcome {
        AgenticTurnOutcome {
            provider_id: self.provider_id,
            model: self.model,
            text: self.text,
            tool_calls: self.tool_calls,
            usage: self.usage,
            stop_reason: self.stop_reason,
            reasoning_encrypted: self.reasoning_encrypted,
        }
    }
}

async fn parse_tool_arguments(
    tool: &ActiveToolCall,
    sink: &TurnEventSink,
) -> Result<Option<serde_json::Value>, LlmError> {
    if tool.arguments_raw.trim().is_empty() {
        return Ok(None);
    }
    match serde_json::from_str(&tool.arguments_raw) {
        Ok(arguments) => Ok(Some(arguments)),
        Err(err) => {
            let message = format!(
                "malformed tool arguments for `{}` ({}): {err}",
                tool.name, tool.id
            );
            sink.emit(AgenticTurnEvent::ProviderError {
                message: message.clone(),
            })
            .await?;
            Err(LlmError::Serialize(message))
        }
    }
}
