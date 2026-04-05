/// OpenAI provider adapter implementing `LlmProvider`.
///
/// Translates between the provider-agnostic `LlmRequest`/`LlmResponse` types
/// and the OpenAI Chat Completions API format.
use async_trait::async_trait;
use tokio::sync::mpsc::Receiver;

use crate::provider::{LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo, ProviderFeature};
use crate::streaming::StreamEvent;
use crate::types::{ContentBlockType, Usage};

// ---------------------------------------------------------------------------
// OpenAiProvider
// ---------------------------------------------------------------------------

pub struct OpenAiProvider {
    /// Resolved API key (env var takes priority over config).
    api_key: String,
    /// Base URL override (defaults to https://api.openai.com/v1).
    base_url: String,
    http: reqwest::Client,
}

impl OpenAiProvider {
    /// Create a new OpenAI provider.
    ///
    /// `api_key` is the fallback if `OPENAI_API_KEY` env var is not set.
    pub fn new(api_key: String, base_url: Option<String>, _default_model: String) -> Self {
        let resolved_key = Self::resolve_api_key(&api_key);
        let resolved_url = base_url.unwrap_or_else(|| "https://api.openai.com/v1".to_string());
        Self {
            api_key: resolved_key,
            base_url: resolved_url,
            http: reqwest::Client::new(),
        }
    }

    /// Resolve the API key: env var wins, then config fallback.
    pub fn resolve_api_key(config_key: &str) -> String {
        std::env::var("OPENAI_API_KEY").unwrap_or_else(|_| config_key.to_string())
    }

    /// Convert Archon system blocks and messages into OpenAI message array.
    ///
    /// System blocks are joined and prepended as `{"role":"system","content":"..."}`.
    pub fn build_openai_messages(
        system: &[serde_json::Value],
        messages: &[serde_json::Value],
    ) -> Vec<serde_json::Value> {
        let mut result = Vec::new();

        // Collect system text.
        let system_text: String = system
            .iter()
            .filter_map(|block| {
                block
                    .get("text")
                    .and_then(|t| t.as_str())
                    .map(|s| s.to_string())
            })
            .collect::<Vec<_>>()
            .join("\n");

        if !system_text.is_empty() {
            result.push(serde_json::json!({
                "role": "system",
                "content": system_text
            }));
        }

        // Pass-through messages, remapping tool_result blocks.
        for msg in messages {
            let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("user");

            // Check if this message has tool_result content blocks.
            if let Some(content_arr) = msg.get("content").and_then(|c| c.as_array()) {
                // Check for tool_result blocks — these need to become separate tool-role messages.
                let tool_results: Vec<&serde_json::Value> = content_arr
                    .iter()
                    .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_result"))
                    .collect();

                if !tool_results.is_empty() {
                    for tr in tool_results {
                        let tool_call_id =
                            tr.get("tool_use_id").and_then(|t| t.as_str()).unwrap_or("");
                        let content_str = tr.get("content").and_then(|c| c.as_str()).unwrap_or("");
                        result.push(serde_json::json!({
                            "role": "tool",
                            "tool_call_id": tool_call_id,
                            "content": content_str
                        }));
                    }
                    continue;
                }

                // Regular message — pass through content as string if single text block.
                let text_content: Option<String> = if content_arr.len() == 1 {
                    content_arr[0]
                        .get("text")
                        .and_then(|t| t.as_str())
                        .map(|s| s.to_string())
                } else {
                    None
                };

                if let Some(text) = text_content {
                    result.push(serde_json::json!({
                        "role": role,
                        "content": text
                    }));
                } else {
                    result.push(msg.clone());
                }
            } else {
                result.push(msg.clone());
            }
        }

        result
    }

    /// Map Archon tools to OpenAI function-calling format.
    pub fn map_tools_to_openai(tools: &[serde_json::Value]) -> Vec<serde_json::Value> {
        tools
            .iter()
            .map(|tool| {
                let name = tool.get("name").cloned().unwrap_or(serde_json::Value::Null);
                let description = tool
                    .get("description")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                let parameters = tool
                    .get("input_schema")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({"type": "object", "properties": {}}));

                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": name,
                        "description": description,
                        "parameters": parameters
                    }
                })
            })
            .collect()
    }

    /// Parse a single SSE data chunk from OpenAI into zero or more StreamEvents.
    pub fn parse_sse_chunk(chunk: &str) -> Vec<StreamEvent> {
        parse_openai_sse_chunk(chunk)
    }

    /// Build and send the streaming request, return the mpsc receiver.
    async fn do_stream(&self, request: LlmRequest) -> Result<Receiver<StreamEvent>, LlmError> {
        let body = build_openai_request_body(
            &request.model,
            request.max_tokens,
            &request.system,
            &request.messages,
            &request.tools,
            true,
        );

        let url = format!("{}/chat/completions", self.base_url);
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::Http(e.to_string()))?;

        let status = resp.status().as_u16();
        if status >= 400 {
            let msg = resp
                .text()
                .await
                .unwrap_or_else(|_| String::from("unknown"));
            return Err(map_http_error(status, msg));
        }

        let (tx, rx) = tokio::sync::mpsc::channel::<StreamEvent>(256);
        let mut byte_stream = resp.bytes_stream();

        tokio::spawn(async move {
            use futures_util::StreamExt;
            let mut buffer = String::new();

            while let Some(chunk_result) = byte_stream.next().await {
                let chunk_bytes = match chunk_result {
                    Ok(b) => b,
                    Err(e) => {
                        let _ = tx
                            .send(StreamEvent::Error {
                                error_type: "http_error".to_string(),
                                message: e.to_string(),
                            })
                            .await;
                        return;
                    }
                };

                buffer.push_str(&String::from_utf8_lossy(&chunk_bytes));

                // Process complete lines from the buffer.
                while let Some(newline_pos) = buffer.find('\n') {
                    let line = buffer[..newline_pos].trim().to_string();
                    buffer.drain(..=newline_pos);

                    if line.is_empty() {
                        continue;
                    }

                    if line == "data: [DONE]" {
                        let _ = tx.send(StreamEvent::MessageStop).await;
                        return;
                    }

                    if let Some(data) = line.strip_prefix("data: ") {
                        for event in parse_openai_sse_chunk(data) {
                            if tx.send(event).await.is_err() {
                                return;
                            }
                        }
                    }
                }
            }
        });

        Ok(rx)
    }
}

// ---------------------------------------------------------------------------
// Shared request body builder (used by both OpenAiProvider and LocalProvider)
// ---------------------------------------------------------------------------

/// Build an OpenAI-format chat completions request body.
///
/// Exported so that `LocalProvider` can reuse it without duplicating logic.
pub fn build_openai_request_body(
    model: &str,
    max_tokens: u32,
    system: &[serde_json::Value],
    messages: &[serde_json::Value],
    tools: &[serde_json::Value],
    stream: bool,
) -> serde_json::Value {
    let openai_messages = OpenAiProvider::build_openai_messages(system, messages);
    let openai_tools = OpenAiProvider::map_tools_to_openai(tools);

    let mut body = serde_json::json!({
        "model": model,
        "max_tokens": max_tokens,
        "messages": openai_messages,
        "stream": stream
    });

    if !openai_tools.is_empty() {
        body["tools"] = serde_json::Value::Array(openai_tools);
    }

    body
}

// ---------------------------------------------------------------------------
// SSE parsing (shared with LocalProvider)
// ---------------------------------------------------------------------------

/// Parse a single OpenAI SSE JSON chunk into StreamEvents.
///
/// Handles text deltas, tool call starts/argument chunks, and finish reasons.
pub(crate) fn parse_openai_sse_chunk(chunk: &str) -> Vec<StreamEvent> {
    let value: serde_json::Value = match serde_json::from_str(chunk) {
        Ok(v) => v,
        Err(_) => return vec![],
    };

    let choices = match value.get("choices").and_then(|c| c.as_array()) {
        Some(arr) if !arr.is_empty() => arr,
        _ => return vec![],
    };

    let choice = &choices[0];
    let delta = match choice.get("delta") {
        Some(d) => d,
        None => return vec![],
    };

    let finish_reason = choice
        .get("finish_reason")
        .and_then(|fr| fr.as_str())
        .unwrap_or("");

    let mut events = Vec::new();

    // Text content delta.
    if let Some(content) = delta.get("content").and_then(|c| c.as_str()) {
        if !content.is_empty() {
            events.push(StreamEvent::ContentBlockStart {
                index: 0,
                block_type: ContentBlockType::Text,
                tool_use_id: None,
                tool_name: None,
            });
            events.push(StreamEvent::TextDelta {
                index: 0,
                text: content.to_string(),
            });
        }
    }

    // Tool call deltas.
    if let Some(tool_calls) = delta.get("tool_calls").and_then(|tc| tc.as_array()) {
        for tc in tool_calls {
            let tc_index = tc.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as u32;
            let tc_id = tc.get("id").and_then(|i| i.as_str()).map(|s| s.to_string());
            let func = tc.get("function");

            let func_name = func
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                .map(|s| s.to_string());
            let func_args = func
                .and_then(|f| f.get("arguments"))
                .and_then(|a| a.as_str())
                .map(|s| s.to_string());

            // If we have an id and name, this is the start of a new tool call.
            if tc_id.is_some() && func_name.is_some() {
                events.push(StreamEvent::ContentBlockStart {
                    index: tc_index,
                    block_type: ContentBlockType::ToolUse,
                    tool_use_id: tc_id,
                    tool_name: func_name,
                });
            }

            // Argument chunk.
            if let Some(args) = func_args {
                if !args.is_empty() {
                    events.push(StreamEvent::InputJsonDelta {
                        index: tc_index,
                        partial_json: args,
                    });
                }
            }
        }
    }

    // Finish reason handling.
    match finish_reason {
        "tool_calls" => {
            events.push(StreamEvent::ContentBlockStop { index: 0 });
            events.push(StreamEvent::MessageDelta {
                stop_reason: Some("tool_use".to_string()),
                usage: None,
            });
        }
        "stop" => {
            events.push(StreamEvent::MessageDelta {
                stop_reason: Some("end_turn".to_string()),
                usage: None,
            });
        }
        _ => {}
    }

    events
}

// ---------------------------------------------------------------------------
// HTTP error mapping
// ---------------------------------------------------------------------------

fn map_http_error(status: u16, body: String) -> LlmError {
    match status {
        401 => LlmError::Auth(body),
        429 => LlmError::RateLimited {
            retry_after_secs: 60,
        },
        500 | 503 => LlmError::Overloaded,
        _ => LlmError::Server {
            status,
            message: body,
        },
    }
}

// ---------------------------------------------------------------------------
// LlmProvider impl
// ---------------------------------------------------------------------------

#[async_trait]
impl LlmProvider for OpenAiProvider {
    fn name(&self) -> &str {
        "openai"
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![
            ModelInfo {
                id: "gpt-4o".to_string(),
                display_name: "GPT-4o".to_string(),
                context_window: 128_000,
            },
            ModelInfo {
                id: "gpt-4o-mini".to_string(),
                display_name: "GPT-4o mini".to_string(),
                context_window: 128_000,
            },
            ModelInfo {
                id: "gpt-4-turbo".to_string(),
                display_name: "GPT-4 Turbo".to_string(),
                context_window: 128_000,
            },
            ModelInfo {
                id: "o1".to_string(),
                display_name: "o1".to_string(),
                context_window: 200_000,
            },
            ModelInfo {
                id: "o3-mini".to_string(),
                display_name: "o3-mini".to_string(),
                context_window: 200_000,
            },
        ]
    }

    async fn stream(&self, request: LlmRequest) -> Result<Receiver<StreamEvent>, LlmError> {
        self.do_stream(request).await
    }

    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse, LlmError> {
        let mut rx = self.stream(request).await?;

        let mut text_parts: Vec<String> = Vec::new();
        let mut usage = Usage::default();
        let mut stop_reason = String::new();

        while let Some(event) = rx.recv().await {
            match event {
                StreamEvent::MessageStart { usage: u, .. } => {
                    usage.merge(&u);
                }
                StreamEvent::TextDelta { text, .. } => {
                    text_parts.push(text);
                }
                StreamEvent::MessageDelta {
                    stop_reason: Some(sr),
                    ..
                } => {
                    stop_reason = sr;
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
            ProviderFeature::ToolUse
                | ProviderFeature::Streaming
                | ProviderFeature::SystemPrompt
                | ProviderFeature::Vision
        )
    }
}
