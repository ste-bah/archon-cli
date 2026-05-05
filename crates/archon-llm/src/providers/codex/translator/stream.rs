use std::collections::{HashMap, HashSet};

use crate::provider::LlmError;
use crate::providers::codex::types::{ResponseOutputItem, ResponseStreamEvent, ResponseUsage};
use crate::streaming::StreamEvent;
use crate::types::{ContentBlockType, Usage};

#[derive(Debug, Default)]
pub struct StreamAccumulator {
    indexes: HashMap<String, u32>,
    items_with_argument_delta: HashSet<String>,
    captured_reasoning_blob: Option<String>,
    next_block_index: u32,
}

impl StreamAccumulator {
    pub fn process(&mut self, event: ResponseStreamEvent) -> Vec<Result<StreamEvent, LlmError>> {
        match event {
            ResponseStreamEvent::Created { response } => vec![Ok(StreamEvent::MessageStart {
                id: response.id,
                model: response.model.unwrap_or_default(),
                usage: Usage::default(),
            })],
            ResponseStreamEvent::OutputItemAdded { item, .. } => self.output_item_added(item),
            ResponseStreamEvent::OutputTextDelta { item_id, delta, .. } => {
                self.indexed(item_id, |index| StreamEvent::TextDelta {
                    index,
                    text: delta,
                })
            }
            ResponseStreamEvent::OutputTextDone { item_id, .. }
            | ResponseStreamEvent::ReasoningDone { item_id, .. } => {
                self.indexed(item_id, |index| StreamEvent::ContentBlockStop { index })
            }
            ResponseStreamEvent::ReasoningDelta { item_id, delta, .. } => {
                self.indexed(item_id, |index| StreamEvent::ThinkingDelta {
                    index,
                    thinking: delta,
                })
            }
            ResponseStreamEvent::FunctionCallArgumentsDelta { item_id, delta, .. } => {
                self.items_with_argument_delta.insert(item_id.clone());
                self.indexed(item_id, |index| StreamEvent::InputJsonDelta {
                    index,
                    partial_json: delta,
                })
            }
            ResponseStreamEvent::FunctionCallArgumentsDone {
                item_id, arguments, ..
            } => self.function_call_arguments_done(item_id, arguments),
            ResponseStreamEvent::OutputItemDone { item, .. } => {
                if let ResponseOutputItem::Reasoning {
                    encrypted_content: Some(blob),
                    ..
                } = item
                {
                    self.captured_reasoning_blob = Some(blob);
                }
                Vec::new()
            }
            ResponseStreamEvent::Completed { response } => {
                self.completed(response.usage, response.status)
            }
            ResponseStreamEvent::Failed { response }
            | ResponseStreamEvent::Incomplete { response } => {
                let (error_type, message) = response
                    .error
                    .map(|e| (e.code, e.message))
                    .unwrap_or_else(|| ("response_failed".into(), "Codex response failed".into()));
                vec![Ok(StreamEvent::Error {
                    error_type,
                    message,
                })]
            }
            ResponseStreamEvent::Error { code, message, .. } => {
                vec![Ok(StreamEvent::Error {
                    error_type: code,
                    message,
                })]
            }
            ResponseStreamEvent::RefusalDelta { item_id, delta, .. } => {
                self.indexed(item_id, |index| StreamEvent::TextDelta {
                    index,
                    text: format!("[REFUSAL]: {delta}"),
                })
            }
            ResponseStreamEvent::InProgress { .. }
            | ResponseStreamEvent::ContentPartAdded { .. }
            | ResponseStreamEvent::ContentPartDone { .. }
            | ResponseStreamEvent::ReasoningSummaryDelta { .. }
            | ResponseStreamEvent::ReasoningSummaryDone { .. }
            | ResponseStreamEvent::Unknown => Vec::new(),
        }
    }

    fn output_item_added(
        &mut self,
        item: ResponseOutputItem,
    ) -> Vec<Result<StreamEvent, LlmError>> {
        let index = self.next_block_index;
        self.next_block_index += 1;
        match item {
            ResponseOutputItem::Message { id, .. } => {
                self.indexes.insert(id, index);
                vec![Ok(StreamEvent::ContentBlockStart {
                    index,
                    block_type: ContentBlockType::Text,
                    tool_use_id: None,
                    tool_name: None,
                })]
            }
            ResponseOutputItem::FunctionCall {
                id, call_id, name, ..
            } => {
                self.indexes.insert(id, index);
                vec![Ok(StreamEvent::ContentBlockStart {
                    index,
                    block_type: ContentBlockType::ToolUse,
                    tool_use_id: Some(call_id),
                    tool_name: Some(name),
                })]
            }
            ResponseOutputItem::Reasoning { id, .. } => {
                self.indexes.insert(id, index);
                vec![Ok(StreamEvent::ContentBlockStart {
                    index,
                    block_type: ContentBlockType::Thinking,
                    tool_use_id: None,
                    tool_name: None,
                })]
            }
            ResponseOutputItem::Unknown => Vec::new(),
        }
    }

    fn indexed<F>(&self, item_id: String, f: F) -> Vec<Result<StreamEvent, LlmError>>
    where
        F: FnOnce(u32) -> StreamEvent,
    {
        self.indexes
            .get(&item_id)
            .copied()
            .map(|index| vec![Ok(f(index))])
            .unwrap_or_default()
    }

    fn completed(
        &mut self,
        usage: Option<ResponseUsage>,
        status: Option<String>,
    ) -> Vec<Result<StreamEvent, LlmError>> {
        let mut events = vec![Ok(StreamEvent::MessageDelta {
            stop_reason: status,
            usage: usage.map(usage_into_archon),
        })];
        if let Some(blob) = self.captured_reasoning_blob.take() {
            events.push(Ok(StreamEvent::ReasoningEncrypted { blob }));
        }
        events.push(Ok(StreamEvent::MessageStop));
        events
    }

    fn function_call_arguments_done(
        &mut self,
        item_id: String,
        arguments: String,
    ) -> Vec<Result<StreamEvent, LlmError>> {
        let Some(index) = self.indexes.get(&item_id).copied() else {
            return Vec::new();
        };
        let saw_delta = self.items_with_argument_delta.remove(&item_id);
        let mut events = Vec::new();
        if !saw_delta && !arguments.is_empty() {
            events.push(Ok(StreamEvent::InputJsonDelta {
                index,
                partial_json: arguments,
            }));
        }
        events.push(Ok(StreamEvent::ContentBlockStop { index }));
        events
    }
}

pub fn process_responses_stream(
    events: impl IntoIterator<Item = ResponseStreamEvent>,
) -> Vec<Result<StreamEvent, LlmError>> {
    let mut accumulator = StreamAccumulator::default();
    events
        .into_iter()
        .flat_map(|event| accumulator.process(event))
        .collect()
}

fn usage_into_archon(usage: ResponseUsage) -> Usage {
    Usage {
        input_tokens: usage.input_tokens as u64,
        output_tokens: usage.output_tokens as u64,
        cache_creation_input_tokens: 0,
        cache_read_input_tokens: usage
            .input_tokens_details
            .and_then(|d| d.cached_tokens)
            .unwrap_or(0) as u64,
    }
}
