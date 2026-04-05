/// AWS Bedrock Converse API provider implementing `LlmProvider`.
///
/// Uses the Bedrock Converse streaming API with SigV4 request signing.
/// Supports all Bedrock-hosted models; Claude models get additional feature flags.
use async_trait::async_trait;
use tokio::sync::mpsc::Receiver;

use reqwest::Url;

use crate::provider::{LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo, ProviderFeature};
use crate::providers::aws_auth::{resolve_credentials, signed_headers};
use crate::streaming::StreamEvent;
use crate::types::{ContentBlockType, Usage};

// ---------------------------------------------------------------------------
// BedrockProvider
// ---------------------------------------------------------------------------

pub struct BedrockProvider {
    region: String,
    model_id: String,
    http: reqwest::Client,
}

impl BedrockProvider {
    /// Create a new Bedrock provider.
    pub fn new(region: String, model_id: String) -> Self {
        Self {
            region,
            model_id,
            http: reqwest::Client::new(),
        }
    }

    /// Build the Bedrock Converse request body.
    pub fn build_converse_body(
        system: &[serde_json::Value],
        messages: &[serde_json::Value],
        tools: &[serde_json::Value],
        max_tokens: u32,
    ) -> serde_json::Value {
        // System prompt: array of {text: "..."} objects.
        let system_arr: Vec<serde_json::Value> = system
            .iter()
            .filter_map(|block| {
                block
                    .get("text")
                    .and_then(|t| t.as_str())
                    .map(|text| serde_json::json!({"text": text}))
            })
            .collect();

        // Messages: convert Archon format to Bedrock Converse format.
        let bedrock_messages: Vec<serde_json::Value> = messages
            .iter()
            .map(|msg| convert_message_to_bedrock(msg))
            .collect();

        let mut body = serde_json::json!({
            "inferenceConfig": {
                "maxTokens": max_tokens
            },
            "messages": bedrock_messages
        });

        if !system_arr.is_empty() {
            body["system"] = serde_json::Value::Array(system_arr);
        }

        if !tools.is_empty() {
            let bedrock_tools: Vec<serde_json::Value> = tools
                .iter()
                .map(|tool| {
                    let name = tool.get("name").cloned().unwrap_or(serde_json::Value::Null);
                    let description = tool
                        .get("description")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    let input_schema = tool
                        .get("input_schema")
                        .cloned()
                        .unwrap_or_else(|| serde_json::json!({"type": "object"}));
                    serde_json::json!({
                        "toolSpec": {
                            "name": name,
                            "description": description,
                            "inputSchema": {
                                "json": input_schema
                            }
                        }
                    })
                })
                .collect();

            body["toolConfig"] = serde_json::json!({"tools": bedrock_tools});
        }

        body
    }

    /// Build the Bedrock runtime URL for this provider.
    fn endpoint_url(&self) -> String {
        format!(
            "https://bedrock-runtime.{}.amazonaws.com/model/{}/converse-stream",
            self.region,
            urlencoding::encode(&self.model_id)
        )
    }

    fn is_claude_model(&self) -> bool {
        self.model_id.starts_with("anthropic.")
    }

    async fn do_stream(&self, request: LlmRequest) -> Result<Receiver<StreamEvent>, LlmError> {
        let creds = resolve_credentials()?;

        let body = Self::build_converse_body(
            &request.system,
            &request.messages,
            &request.tools,
            request.max_tokens,
        );

        let body_bytes =
            serde_json::to_vec(&body).map_err(|e| LlmError::Serialize(e.to_string()))?;

        let url = self.endpoint_url();
        let url_parsed =
            Url::parse(&url).map_err(|e| LlmError::Http(format!("invalid URL: {e}")))?;
        let host = url_parsed.host_str().unwrap_or("").to_string();
        let path = url_parsed.path().to_string();

        let (x_amz_date, authorization) =
            signed_headers(&creds, &host, &path, &self.region, &body_bytes);

        let resp = self
            .http
            .post(&url)
            .header("content-type", "application/json")
            .header("x-amz-date", &x_amz_date)
            .header("authorization", &authorization)
            .body(body_bytes)
            .send()
            .await
            .map_err(|e| LlmError::Http(e.to_string()))?;

        let status = resp.status().as_u16();
        if status >= 400 {
            let msg = resp.text().await.unwrap_or_else(|_| "unknown".to_string());
            return Err(map_http_error(status, msg));
        }

        let (tx, rx) = tokio::sync::mpsc::channel::<StreamEvent>(256);
        let mut byte_stream = resp.bytes_stream();

        tokio::spawn(async move {
            use futures_util::StreamExt;
            let mut buffer = Vec::new();

            while let Some(chunk_result) = byte_stream.next().await {
                let chunk = match chunk_result {
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
                buffer.extend_from_slice(&chunk);

                // Bedrock sends JSON events. Try to extract complete JSON objects.
                let text = String::from_utf8_lossy(&buffer).to_string();
                let (events, consumed) = extract_bedrock_events(&text);
                if consumed > 0 {
                    buffer.drain(..consumed);
                }

                for event in events {
                    for stream_event in parse_bedrock_event(&event) {
                        if tx.send(stream_event).await.is_err() {
                            return;
                        }
                    }
                }
            }

            // Drain any remaining buffer content.
            if !buffer.is_empty() {
                let text = String::from_utf8_lossy(&buffer).to_string();
                let (events, _) = extract_bedrock_events(&text);
                for event in events {
                    for stream_event in parse_bedrock_event(&event) {
                        if tx.send(stream_event).await.is_err() {
                            return;
                        }
                    }
                }
            }
        });

        Ok(rx)
    }
}

// ---------------------------------------------------------------------------
// Message conversion: Archon → Bedrock Converse format
// ---------------------------------------------------------------------------

fn convert_message_to_bedrock(msg: &serde_json::Value) -> serde_json::Value {
    let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("user");

    // Map content blocks.
    if let Some(content_arr) = msg.get("content").and_then(|c| c.as_array()) {
        let bedrock_content: Vec<serde_json::Value> = content_arr
            .iter()
            .filter_map(|block| convert_content_block(block))
            .collect();

        serde_json::json!({
            "role": role,
            "content": bedrock_content
        })
    } else if let Some(content_str) = msg.get("content").and_then(|c| c.as_str()) {
        serde_json::json!({
            "role": role,
            "content": [{"text": content_str}]
        })
    } else {
        serde_json::json!({
            "role": role,
            "content": []
        })
    }
}

fn convert_content_block(block: &serde_json::Value) -> Option<serde_json::Value> {
    let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
    match block_type {
        "text" => {
            let text = block.get("text").and_then(|t| t.as_str()).unwrap_or("");
            Some(serde_json::json!({"text": text}))
        }
        "tool_use" => {
            let id = block.get("id").and_then(|i| i.as_str()).unwrap_or("");
            let name = block.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let input = block
                .get("input")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            Some(serde_json::json!({
                "toolUse": {
                    "toolUseId": id,
                    "name": name,
                    "input": input
                }
            }))
        }
        "tool_result" => {
            let tool_use_id = block
                .get("tool_use_id")
                .and_then(|t| t.as_str())
                .unwrap_or("");
            let content = block.get("content").and_then(|c| c.as_str()).unwrap_or("");
            Some(serde_json::json!({
                "toolResult": {
                    "toolUseId": tool_use_id,
                    "content": [{"text": content}]
                }
            }))
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Bedrock event parsing
// ---------------------------------------------------------------------------

/// Extract complete JSON objects from a buffer of text.
/// Returns (events, bytes_consumed).
fn extract_bedrock_events(text: &str) -> (Vec<serde_json::Value>, usize) {
    let mut events = Vec::new();
    let mut consumed = 0;
    let bytes = text.as_bytes();
    let mut pos = 0;

    while pos < bytes.len() {
        // Skip whitespace and newlines.
        while pos < bytes.len()
            && (bytes[pos] == b'\r' || bytes[pos] == b'\n' || bytes[pos] == b' ')
        {
            pos += 1;
        }

        if pos >= bytes.len() {
            break;
        }

        // Find a JSON object (starting with '{').
        if bytes[pos] != b'{' {
            break;
        }

        // Try to find the end of this JSON object.
        if let Some(end) = find_json_object_end(bytes, pos) {
            let slice = &text[pos..=end];
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(slice) {
                events.push(val);
                consumed = end + 1;
            }
            pos = end + 1;
        } else {
            // Incomplete JSON — stop here.
            break;
        }
    }

    (events, consumed)
}

/// Find the end index of a JSON object starting at `start`.
fn find_json_object_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape_next = false;
    let mut i = start;

    while i < bytes.len() {
        let b = bytes[i];

        if escape_next {
            escape_next = false;
            i += 1;
            continue;
        }

        if in_string {
            match b {
                b'\\' => escape_next = true,
                b'"' => in_string = false,
                _ => {}
            }
        } else {
            match b {
                b'"' => in_string = true,
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i);
                    }
                }
                _ => {}
            }
        }

        i += 1;
    }

    None
}

/// Parse a Bedrock Converse stream event JSON value into StreamEvent(s).
pub fn parse_bedrock_event(event: &serde_json::Value) -> Vec<StreamEvent> {
    let mut events = Vec::new();

    if let Some(start) = event.get("contentBlockStart") {
        let index = start
            .get("contentBlockIndex")
            .and_then(|i| i.as_u64())
            .unwrap_or(0) as u32;
        let block = start.get("contentBlock");
        let has_tool_use = block.and_then(|b| b.get("toolUse")).is_some();

        if has_tool_use {
            let tool_use = block.and_then(|b| b.get("toolUse"));
            let tool_id = tool_use
                .and_then(|t| t.get("toolUseId"))
                .and_then(|i| i.as_str())
                .map(|s| s.to_string());
            let tool_name = tool_use
                .and_then(|t| t.get("name"))
                .and_then(|n| n.as_str())
                .map(|s| s.to_string());
            events.push(StreamEvent::ContentBlockStart {
                index,
                block_type: ContentBlockType::ToolUse,
                tool_use_id: tool_id,
                tool_name,
            });
        } else {
            events.push(StreamEvent::ContentBlockStart {
                index,
                block_type: ContentBlockType::Text,
                tool_use_id: None,
                tool_name: None,
            });
        }
    }

    if let Some(delta_obj) = event.get("contentBlockDelta") {
        let index = delta_obj
            .get("contentBlockIndex")
            .and_then(|i| i.as_u64())
            .unwrap_or(0) as u32;
        let delta = delta_obj.get("delta");

        if let Some(text) = delta.and_then(|d| d.get("text")).and_then(|t| t.as_str()) {
            events.push(StreamEvent::TextDelta {
                index,
                text: text.to_string(),
            });
        } else if let Some(json_str) = delta
            .and_then(|d| d.get("toolUse"))
            .and_then(|t| t.get("input"))
            .and_then(|i| i.as_str())
        {
            events.push(StreamEvent::InputJsonDelta {
                index,
                partial_json: json_str.to_string(),
            });
        }
    }

    if let Some(stop_obj) = event.get("contentBlockStop") {
        let index = stop_obj
            .get("contentBlockIndex")
            .and_then(|i| i.as_u64())
            .unwrap_or(0) as u32;
        events.push(StreamEvent::ContentBlockStop { index });
    }

    if let Some(msg_delta) = event.get("messageStop") {
        let stop_reason = msg_delta
            .get("stopReason")
            .and_then(|r| r.as_str())
            .map(|s| {
                // Normalize Bedrock stop reasons to Anthropic conventions.
                match s {
                    "end_turn" => "end_turn",
                    "tool_use" => "tool_use",
                    "max_tokens" => "max_tokens",
                    other => other,
                }
                .to_string()
            });
        events.push(StreamEvent::MessageDelta {
            stop_reason,
            usage: None,
        });
        events.push(StreamEvent::MessageStop);
    }

    if let Some(metadata) = event.get("metadata") {
        if let Some(usage) = metadata.get("usage") {
            let input_tokens = usage
                .get("inputTokens")
                .and_then(|t| t.as_u64())
                .unwrap_or(0);
            let output_tokens = usage
                .get("outputTokens")
                .and_then(|t| t.as_u64())
                .unwrap_or(0);
            events.push(StreamEvent::MessageDelta {
                stop_reason: None,
                usage: Some(Usage {
                    input_tokens,
                    output_tokens,
                    cache_creation_input_tokens: 0,
                    cache_read_input_tokens: 0,
                }),
            });
        }
    }

    events
}

// ---------------------------------------------------------------------------
// HTTP error mapping
// ---------------------------------------------------------------------------

fn map_http_error(status: u16, body: String) -> LlmError {
    match status {
        400 => LlmError::Server {
            status,
            message: format!("Bad request: {body}"),
        },
        401 | 403 => LlmError::Auth(body),
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
impl LlmProvider for BedrockProvider {
    fn name(&self) -> &str {
        "bedrock"
    }

    fn models(&self) -> Vec<ModelInfo> {
        // Return the configured model as the available model.
        vec![ModelInfo {
            id: self.model_id.clone(),
            display_name: self.model_id.clone(),
            context_window: 200_000,
        }]
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
                    usage: Some(u),
                } => {
                    stop_reason = sr;
                    usage.merge(&u);
                }
                StreamEvent::MessageDelta {
                    stop_reason: Some(sr),
                    ..
                } => {
                    stop_reason = sr;
                }
                StreamEvent::MessageDelta { usage: Some(u), .. } => {
                    usage.merge(&u);
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
        let is_claude = self.is_claude_model();
        match feature {
            ProviderFeature::Thinking | ProviderFeature::PromptCaching => is_claude,
            ProviderFeature::ToolUse
            | ProviderFeature::Streaming
            | ProviderFeature::SystemPrompt => true,
            ProviderFeature::Vision => is_claude,
        }
    }
}
