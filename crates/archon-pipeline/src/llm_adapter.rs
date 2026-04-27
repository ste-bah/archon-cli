//! LLM client adapter — bridges `archon_llm::AnthropicClient` to the
//! pipeline's [`LlmClient`] trait.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

use archon_llm::anthropic::{AnthropicClient, MessageRequest};
use archon_llm::streaming::StreamEvent;

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
            request_origin: Some("pipeline".into()),
        };

        let mut rx = self
            .client
            .stream_message(request)
            .await
            .map_err(|e| anyhow::anyhow!("LLM API error: {e}"))?;

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
                | StreamEvent::MessageStop
                | StreamEvent::Ping
                | StreamEvent::Error { .. } => {}
            }
        }

        Ok(LlmResponse {
            content: text_parts.join(""),
            tool_uses,
            tokens_in,
            tokens_out,
        })
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

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
}
