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
    let cache_read_input_tokens = usage
        .input_tokens_details
        .and_then(|details| details.cached_tokens);
    // `None` and `Some(0)` mean different things and the counters cannot tell
    // them apart: the first is "the service reported nothing", the second is
    // "the service reported a miss". Both surface as a bare zero in the status
    // bar, so the distinction only exists here.
    tracing::debug!(
        provider = "openai-codex",
        input_tokens = usage.input_tokens,
        output_tokens = usage.output_tokens,
        cached_tokens = ?cache_read_input_tokens,
        "Codex usage"
    );

    // OpenAI reports `input_tokens` as the TOTAL, with `cached_tokens` a subset
    // of it. Anthropic and Bedrock report the two as disjoint — Bedrock's
    // `inputTokens` excludes the cached ones entirely, verified live at
    // `inputTokens: 3` on a 4,424-token request — and `UsageAccumulator` sums
    // the buckets to get the context size, so it assumes the disjoint form.
    //
    // Passing OpenAI's total through unchanged therefore counted every cached
    // token twice: once as billable input and once as a cache read. Measured on
    // a ~12k prompt, that reported 21,949 tokens of context. It inflated the
    // context-pressure figure, which is what sizes auto-compaction, and
    // overcharged the cost estimate by pricing the cached tokens at the full
    // input rate on top of the cache-read rate — making a working cache look
    // more expensive than no cache at all.
    let cached = cache_read_input_tokens.unwrap_or(0) as u64;
    let billable_input = (usage.input_tokens as u64).saturating_sub(cached);

    Usage {
        input_tokens: billable_input,
        output_tokens: usage.output_tokens as u64,
        cache_creation_input_tokens: 0,
        cache_read_input_tokens: cache_read_input_tokens.unwrap_or(0) as u64,
        input_tokens_available: true,
        output_tokens_available: true,
        cache_creation_input_tokens_available: false,
        cache_read_input_tokens_available: cache_read_input_tokens.is_some(),
    }
}

#[cfg(test)]
mod usage_tests {
    use super::*;
    use crate::providers::codex::types::TokenDetails;

    fn usage(input_tokens: u32, cached: Option<u32>) -> ResponseUsage {
        ResponseUsage {
            input_tokens,
            output_tokens: 10,
            total_tokens: input_tokens + 10,
            input_tokens_details: cached.map(|cached_tokens| TokenDetails {
                cached_tokens: Some(cached_tokens),
                reasoning_tokens: None,
            }),
            output_tokens_details: None,
        }
    }

    /// The figures from a live GPT-5.6 turn. OpenAI's `input_tokens` is the
    /// total *including* the cached ones, so the buckets have to be made
    /// disjoint before `UsageAccumulator` sums them into a context size.
    #[test]
    fn cached_tokens_are_removed_from_billable_input() {
        let converted = usage_into_archon(usage(12_221, Some(9_728)));

        assert_eq!(converted.input_tokens, 2_493);
        assert_eq!(converted.cache_read_input_tokens, 9_728);
        assert_eq!(
            converted.input_tokens + converted.cache_read_input_tokens,
            12_221,
            "the two buckets must reconstruct OpenAI's total exactly once"
        );
    }

    #[test]
    fn a_cold_turn_is_unchanged() {
        let converted = usage_into_archon(usage(12_221, Some(0)));

        assert_eq!(converted.input_tokens, 12_221);
        assert_eq!(converted.cache_read_input_tokens, 0);
        assert!(
            converted.cache_read_input_tokens_available,
            "a reported zero is a measured miss, not an absent measurement"
        );
    }

    #[test]
    fn absent_details_leave_the_input_alone_and_report_unavailable() {
        let converted = usage_into_archon(usage(12_221, None));

        assert_eq!(converted.input_tokens, 12_221);
        assert!(!converted.cache_read_input_tokens_available);
    }

    /// Defensive: a service reporting more cached than total must not wrap.
    #[test]
    fn cached_exceeding_the_total_saturates_at_zero() {
        let converted = usage_into_archon(usage(100, Some(500)));

        assert_eq!(converted.input_tokens, 0);
    }
}
