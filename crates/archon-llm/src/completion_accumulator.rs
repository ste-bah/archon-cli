use std::collections::BTreeMap;

use serde_json::json;
use tokio::sync::mpsc::Receiver;

use crate::provider::{LlmError, LlmResponse};
use crate::streaming::StreamEvent;
use crate::types::{ContentBlockType, Usage};

pub(crate) async fn collect_completion_response(
    mut rx: Receiver<StreamEvent>,
) -> Result<LlmResponse, LlmError> {
    let mut accumulator = CompletionAccumulator::default();
    while let Some(event) = rx.recv().await {
        accumulator.process(event)?;
    }
    accumulator.into_response()
}

#[derive(Default)]
struct CompletionAccumulator {
    blocks: BTreeMap<u32, CompletionBlock>,
    usage: Usage,
    stop_reason: String,
}

enum CompletionBlock {
    Text(String),
    Thinking(String),
    ToolUse {
        id: String,
        name: String,
        input_json: String,
    },
}

impl CompletionAccumulator {
    fn process(&mut self, event: StreamEvent) -> Result<(), LlmError> {
        match event {
            StreamEvent::MessageStart { usage, .. } => self.usage.merge(&usage),
            StreamEvent::ContentBlockStart {
                index,
                block_type,
                tool_use_id,
                tool_name,
            } => self.start_block(index, block_type, tool_use_id, tool_name),
            StreamEvent::TextDelta { index, text } => self.text_delta(index, text),
            StreamEvent::ThinkingDelta { index, thinking } => self.thinking_delta(index, thinking),
            StreamEvent::InputJsonDelta {
                index,
                partial_json,
            } => self.input_json_delta(index, partial_json),
            StreamEvent::MessageDelta { usage, stop_reason } => {
                if let Some(usage) = usage {
                    self.usage.merge(&usage);
                }
                if let Some(stop_reason) = stop_reason {
                    self.stop_reason = stop_reason;
                }
            }
            StreamEvent::Error { message, .. } => return Err(LlmError::Http(message)),
            StreamEvent::ContentBlockStop { .. }
            | StreamEvent::MessageStop
            | StreamEvent::Ping
            | StreamEvent::SignatureDelta { .. }
            | StreamEvent::ReasoningEncrypted { .. } => {}
        }
        Ok(())
    }

    fn start_block(
        &mut self,
        index: u32,
        block_type: ContentBlockType,
        tool_use_id: Option<String>,
        tool_name: Option<String>,
    ) {
        if block_type == ContentBlockType::ToolUse
            && let Some(CompletionBlock::ToolUse { id, name, .. }) = self.blocks.get_mut(&index)
        {
            if let Some(tool_use_id) = tool_use_id {
                *id = tool_use_id;
            }
            if let Some(tool_name) = tool_name {
                *name = tool_name;
            }
            return;
        }

        let block = match block_type {
            ContentBlockType::ToolUse => CompletionBlock::ToolUse {
                id: tool_use_id.unwrap_or_else(|| format!("tool-{index}")),
                name: tool_name.unwrap_or_default(),
                input_json: String::new(),
            },
            ContentBlockType::Thinking => CompletionBlock::Thinking(String::new()),
            _ => CompletionBlock::Text(String::new()),
        };
        self.blocks.entry(index).or_insert(block);
    }

    fn text_delta(&mut self, index: u32, text: String) {
        match self
            .blocks
            .entry(index)
            .or_insert_with(|| CompletionBlock::Text(String::new()))
        {
            CompletionBlock::Text(existing) => existing.push_str(&text),
            CompletionBlock::Thinking(existing) => existing.push_str(&text),
            CompletionBlock::ToolUse { .. } => {}
        }
    }

    fn thinking_delta(&mut self, index: u32, thinking: String) {
        match self
            .blocks
            .entry(index)
            .or_insert_with(|| CompletionBlock::Thinking(String::new()))
        {
            CompletionBlock::Thinking(existing) => existing.push_str(&thinking),
            CompletionBlock::Text(existing) => existing.push_str(&thinking),
            CompletionBlock::ToolUse { .. } => {}
        }
    }

    fn input_json_delta(&mut self, index: u32, partial_json: String) {
        match self
            .blocks
            .entry(index)
            .or_insert_with(|| CompletionBlock::ToolUse {
                id: format!("tool-{index}"),
                name: String::new(),
                input_json: String::new(),
            }) {
            CompletionBlock::ToolUse { input_json, .. } => input_json.push_str(&partial_json),
            CompletionBlock::Text(_) | CompletionBlock::Thinking(_) => {}
        }
    }

    fn into_response(self) -> Result<LlmResponse, LlmError> {
        let mut content = Vec::new();
        for block in self.blocks.into_values() {
            match block {
                CompletionBlock::Text(text) if !text.is_empty() => {
                    content.push(json!({"type": "text", "text": text}));
                }
                CompletionBlock::ToolUse {
                    id,
                    name,
                    input_json,
                } => {
                    let input = parse_tool_input(&id, &name, &input_json)?;
                    content.push(json!({
                        "type": "tool_use",
                        "id": id,
                        "name": name,
                        "input": input,
                    }));
                }
                CompletionBlock::Text(_) | CompletionBlock::Thinking(_) => {}
            }
        }

        Ok(LlmResponse {
            content,
            usage: self.usage,
            stop_reason: self.stop_reason,
        })
    }
}

fn parse_tool_input(
    tool_id: &str,
    tool_name: &str,
    input_json: &str,
) -> Result<serde_json::Value, LlmError> {
    if input_json.trim().is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str(input_json).map_err(|e| {
        LlmError::Serialize(format!(
            "malformed tool arguments for `{tool_name}` ({tool_id}): {e}"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rx_from(events: Vec<StreamEvent>) -> Receiver<StreamEvent> {
        let (tx, rx) = tokio::sync::mpsc::channel(events.len().max(1));
        tokio::spawn(async move {
            for event in events {
                tx.send(event).await.expect("send event");
            }
        });
        rx
    }

    #[tokio::test]
    async fn collects_text_and_tool_blocks() {
        let response = collect_completion_response(rx_from(vec![
            StreamEvent::MessageStart {
                id: "msg_1".into(),
                model: "model".into(),
                usage: Usage {
                    input_tokens: 1,
                    ..Usage::default()
                },
            },
            StreamEvent::ContentBlockStart {
                index: 0,
                block_type: ContentBlockType::Text,
                tool_use_id: None,
                tool_name: None,
            },
            StreamEvent::TextDelta {
                index: 0,
                text: "checking ".into(),
            },
            StreamEvent::ContentBlockStart {
                index: 1,
                block_type: ContentBlockType::ToolUse,
                tool_use_id: Some("toolu_1".into()),
                tool_name: Some("Lookup".into()),
            },
            StreamEvent::InputJsonDelta {
                index: 1,
                partial_json: "{\"query\":\"arc".into(),
            },
            StreamEvent::InputJsonDelta {
                index: 1,
                partial_json: "hon\"}".into(),
            },
            StreamEvent::MessageDelta {
                stop_reason: Some("tool_use".into()),
                usage: Some(Usage {
                    output_tokens: 2,
                    ..Usage::default()
                }),
            },
            StreamEvent::MessageStop,
        ]))
        .await
        .expect("response");

        assert_eq!(response.usage.input_tokens, 1);
        assert_eq!(response.usage.output_tokens, 2);
        assert_eq!(response.stop_reason, "tool_use");
        assert_eq!(
            response.content[0],
            json!({"type": "text", "text": "checking "})
        );
        assert_eq!(
            response.content[1],
            json!({
                "type": "tool_use",
                "id": "toolu_1",
                "name": "Lookup",
                "input": {"query": "archon"},
            })
        );
    }

    #[tokio::test]
    async fn rejects_malformed_tool_arguments() {
        let err = collect_completion_response(rx_from(vec![
            StreamEvent::ContentBlockStart {
                index: 0,
                block_type: ContentBlockType::ToolUse,
                tool_use_id: Some("toolu_bad".into()),
                tool_name: Some("Lookup".into()),
            },
            StreamEvent::InputJsonDelta {
                index: 0,
                partial_json: "{\"query\":".into(),
            },
        ]))
        .await
        .expect_err("malformed arguments should fail");

        assert!(err.to_string().contains("malformed tool arguments"));
        assert!(err.to_string().contains("toolu_bad"));
    }
}
