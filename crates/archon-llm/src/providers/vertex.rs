/// Google Vertex AI provider implementing `LlmProvider`.
///
/// Supports Claude models via the Anthropic Messages API format and
/// Gemini models via the Gemini `contents` API format.
use async_trait::async_trait;
use tokio::sync::mpsc::Receiver;

use crate::provider::{LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo, ProviderFeature};
use crate::providers::gcp_auth::{get_access_token, resolve_credentials, GcpAccessToken};
use crate::streaming::StreamEvent;
use crate::types::{ContentBlockType, Usage};

// ---------------------------------------------------------------------------
// VertexProvider
// ---------------------------------------------------------------------------

pub struct VertexProvider {
    project_id: String,
    region: String,
    model: String,
    publisher: String,
    credentials_file: Option<String>,
    http: reqwest::Client,
    /// Cached access token (protected by Mutex for interior mutability).
    token_cache: tokio::sync::Mutex<Option<GcpAccessToken>>,
}

impl VertexProvider {
    /// Create a new Vertex AI provider.
    ///
    /// - `publisher`: `"anthropic"` for Claude, `"google"` for Gemini.
    /// - `credentials_file`: optional path to service account JSON.
    pub fn new(
        project_id: String,
        region: String,
        model: String,
        publisher: String,
        credentials_file: Option<String>,
    ) -> Self {
        Self {
            project_id,
            region,
            model,
            publisher,
            credentials_file,
            http: reqwest::Client::new(),
            token_cache: tokio::sync::Mutex::new(None),
        }
    }

    /// Build the Vertex AI endpoint URL.
    ///
    /// Format:
    /// `https://{region}-aiplatform.googleapis.com/v1/projects/{project}/locations/{region}/publishers/{publisher}/models/{model}:streamGenerateContent`
    pub fn endpoint_url(&self) -> String {
        format!(
            "https://{region}-aiplatform.googleapis.com/v1/projects/{project}/locations/{region}/publishers/{publisher}/models/{model}:streamGenerateContent",
            region = self.region,
            project = self.project_id,
            publisher = self.publisher,
            model = self.model,
        )
    }

    fn is_claude_model(&self) -> bool {
        self.publisher == "anthropic" || self.model.contains("claude")
    }

    /// Get a valid access token, refreshing if necessary.
    async fn get_token(&self) -> Result<String, LlmError> {
        let mut cache = self.token_cache.lock().await;

        // Check if cached token is still valid (with 60s buffer).
        if let Some(ref token) = *cache {
            if token.expires_at > std::time::Instant::now() {
                return Ok(token.access_token.clone());
            }
        }

        // Fetch fresh token.
        let creds = resolve_credentials(self.credentials_file.as_deref())?;
        let token = get_access_token(&self.http, &creds).await?;
        let access_token = token.access_token.clone();
        *cache = Some(token);
        Ok(access_token)
    }

    /// Build the request body for Claude on Vertex (Anthropic Messages format).
    fn build_claude_body(request: &LlmRequest) -> serde_json::Value {
        let system_text: String = request
            .system
            .iter()
            .filter_map(|b| b.get("text").and_then(|t| t.as_str()).map(|s| s.to_string()))
            .collect::<Vec<_>>()
            .join("\n");

        let mut body = serde_json::json!({
            "anthropic_version": "vertex-2023-10-16",
            "max_tokens": request.max_tokens,
            "messages": request.messages,
            "stream": true
        });

        if !system_text.is_empty() {
            body["system"] = serde_json::Value::String(system_text);
        }

        if !request.tools.is_empty() {
            body["tools"] = serde_json::Value::Array(request.tools.clone());
        }

        body
    }

    /// Build the request body for Gemini on Vertex (`contents` array format).
    fn build_gemini_body(request: &LlmRequest) -> serde_json::Value {
        // Gemini uses a `contents` array with `role` + `parts` structure.
        let contents: Vec<serde_json::Value> = request
            .messages
            .iter()
            .map(|msg| {
                let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("user");
                // Map Anthropic role "assistant" → "model" for Gemini.
                let gemini_role = if role == "assistant" { "model" } else { "user" };

                let parts = if let Some(content_arr) = msg.get("content").and_then(|c| c.as_array()) {
                    content_arr
                        .iter()
                        .filter_map(|block| {
                            block.get("text").and_then(|t| t.as_str()).map(|text| {
                                serde_json::json!({"text": text})
                            })
                        })
                        .collect::<Vec<_>>()
                } else if let Some(text) = msg.get("content").and_then(|c| c.as_str()) {
                    vec![serde_json::json!({"text": text})]
                } else {
                    vec![]
                };

                serde_json::json!({
                    "role": gemini_role,
                    "parts": parts
                })
            })
            .collect();

        serde_json::json!({
            "contents": contents,
            "generationConfig": {
                "maxOutputTokens": request.max_tokens
            }
        })
    }

    async fn do_stream(&self, request: LlmRequest) -> Result<Receiver<StreamEvent>, LlmError> {
        let access_token = self.get_token().await?;
        let url = self.endpoint_url();

        let body = if self.is_claude_model() {
            Self::build_claude_body(&request)
        } else {
            Self::build_gemini_body(&request)
        };

        let resp = self
            .http
            .post(&url)
            .bearer_auth(&access_token)
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::Http(e.to_string()))?;

        let status = resp.status().as_u16();
        if status >= 400 {
            let msg = resp.text().await.unwrap_or_else(|_| "unknown".to_string());
            return Err(map_http_error(status, msg));
        }

        let is_claude = self.is_claude_model();
        let (tx, rx) = tokio::sync::mpsc::channel::<StreamEvent>(256);
        let mut byte_stream = resp.bytes_stream();

        tokio::spawn(async move {
            use futures_util::StreamExt;
            let mut buffer = String::new();

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

                buffer.push_str(&String::from_utf8_lossy(&chunk));

                // Claude on Vertex uses Anthropic SSE format.
                // Gemini on Vertex returns a JSON array of response objects.
                if is_claude {
                    // Process SSE lines.
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
                        if let Some(event_type) = line.strip_prefix("event: ") {
                            let _ = event_type; // stored separately
                        }
                        if let Some(data) = line.strip_prefix("data: ") {
                            if let Ok(val) = serde_json::from_str::<serde_json::Value>(data) {
                                for event in parse_anthropic_vertex_event(&val) {
                                    if tx.send(event).await.is_err() {
                                        return;
                                    }
                                }
                            }
                        }
                    }
                } else {
                    // Gemini returns streamed JSON objects — try to parse.
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(&buffer) {
                        for event in parse_gemini_vertex_event(&val) {
                            if tx.send(event).await.is_err() {
                                return;
                            }
                        }
                        buffer.clear();
                    }
                }
            }

            // Signal end of stream.
            let _ = tx.send(StreamEvent::MessageStop).await;
        });

        Ok(rx)
    }
}

// ---------------------------------------------------------------------------
// Event parsing
// ---------------------------------------------------------------------------

/// Parse Anthropic Messages API SSE event from Vertex AI (same format as direct Anthropic API).
fn parse_anthropic_vertex_event(val: &serde_json::Value) -> Vec<StreamEvent> {
    let event_type = val.get("type").and_then(|t| t.as_str()).unwrap_or("");
    let mut events = Vec::new();

    match event_type {
        "message_start" => {
            let message = val.get("message");
            let id = message
                .and_then(|m| m.get("id"))
                .and_then(|i| i.as_str())
                .unwrap_or("")
                .to_string();
            let model = message
                .and_then(|m| m.get("model"))
                .and_then(|m| m.as_str())
                .unwrap_or("")
                .to_string();
            let usage = parse_usage(message.and_then(|m| m.get("usage")));
            events.push(StreamEvent::MessageStart { id, model, usage });
        }
        "content_block_start" => {
            let index = val.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as u32;
            let block = val.get("content_block");
            let block_type_str = block
                .and_then(|b| b.get("type"))
                .and_then(|t| t.as_str())
                .unwrap_or("text");
            let (block_type, tool_use_id, tool_name) = match block_type_str {
                "tool_use" => {
                    let id = block
                        .and_then(|b| b.get("id"))
                        .and_then(|i| i.as_str())
                        .map(|s| s.to_string());
                    let name = block
                        .and_then(|b| b.get("name"))
                        .and_then(|n| n.as_str())
                        .map(|s| s.to_string());
                    (ContentBlockType::ToolUse, id, name)
                }
                _ => (ContentBlockType::Text, None, None),
            };
            events.push(StreamEvent::ContentBlockStart {
                index,
                block_type,
                tool_use_id,
                tool_name,
            });
        }
        "content_block_delta" => {
            let index = val.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as u32;
            let delta = val.get("delta");
            let delta_type = delta
                .and_then(|d| d.get("type"))
                .and_then(|t| t.as_str())
                .unwrap_or("");
            match delta_type {
                "text_delta" => {
                    let text = delta
                        .and_then(|d| d.get("text"))
                        .and_then(|t| t.as_str())
                        .unwrap_or("")
                        .to_string();
                    events.push(StreamEvent::TextDelta { index, text });
                }
                "input_json_delta" => {
                    let partial_json = delta
                        .and_then(|d| d.get("partial_json"))
                        .and_then(|j| j.as_str())
                        .unwrap_or("")
                        .to_string();
                    events.push(StreamEvent::InputJsonDelta { index, partial_json });
                }
                _ => {}
            }
        }
        "content_block_stop" => {
            let index = val.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as u32;
            events.push(StreamEvent::ContentBlockStop { index });
        }
        "message_delta" => {
            let delta = val.get("delta");
            let stop_reason = delta
                .and_then(|d| d.get("stop_reason"))
                .and_then(|r| r.as_str())
                .map(|s| s.to_string());
            let usage = parse_usage(val.get("usage"));
            events.push(StreamEvent::MessageDelta {
                stop_reason,
                usage: Some(usage),
            });
        }
        "message_stop" => {
            events.push(StreamEvent::MessageStop);
        }
        "ping" => {
            events.push(StreamEvent::Ping);
        }
        _ => {}
    }

    events
}

/// Parse a Gemini streaming response object into StreamEvents.
fn parse_gemini_vertex_event(val: &serde_json::Value) -> Vec<StreamEvent> {
    let mut events = Vec::new();

    if let Some(candidates) = val.get("candidates").and_then(|c| c.as_array()) {
        for candidate in candidates {
            if let Some(content) = candidate.get("content") {
                if let Some(parts) = content.get("parts").and_then(|p| p.as_array()) {
                    for (i, part) in parts.iter().enumerate() {
                        if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                            if !text.is_empty() {
                                events.push(StreamEvent::ContentBlockStart {
                                    index: i as u32,
                                    block_type: ContentBlockType::Text,
                                    tool_use_id: None,
                                    tool_name: None,
                                });
                                events.push(StreamEvent::TextDelta {
                                    index: i as u32,
                                    text: text.to_string(),
                                });
                                events.push(StreamEvent::ContentBlockStop { index: i as u32 });
                            }
                        }
                    }
                }
            }

            let finish_reason = candidate
                .get("finishReason")
                .and_then(|r| r.as_str())
                .unwrap_or("");
            if !finish_reason.is_empty() {
                let stop_reason = match finish_reason {
                    "STOP" => "end_turn",
                    "MAX_TOKENS" => "max_tokens",
                    "SAFETY" => "stop_sequence",
                    other => other,
                };
                events.push(StreamEvent::MessageDelta {
                    stop_reason: Some(stop_reason.to_string()),
                    usage: None,
                });
            }
        }
    }

    events
}

/// Parse a usage object from a Vertex AI response into a `Usage` struct.
fn parse_usage(usage_val: Option<&serde_json::Value>) -> Usage {
    match usage_val {
        Some(u) => Usage {
            input_tokens: u
                .get("input_tokens")
                .and_then(|t| t.as_u64())
                .unwrap_or(0),
            output_tokens: u
                .get("output_tokens")
                .and_then(|t| t.as_u64())
                .unwrap_or(0),
            cache_creation_input_tokens: u
                .get("cache_creation_input_tokens")
                .and_then(|t| t.as_u64())
                .unwrap_or(0),
            cache_read_input_tokens: u
                .get("cache_read_input_tokens")
                .and_then(|t| t.as_u64())
                .unwrap_or(0),
        },
        None => Usage::default(),
    }
}

// ---------------------------------------------------------------------------
// HTTP error mapping
// ---------------------------------------------------------------------------

fn map_http_error(status: u16, body: String) -> LlmError {
    match status {
        401 | 403 => LlmError::Auth(body),
        429 => LlmError::RateLimited { retry_after_secs: 60 },
        500 | 503 => LlmError::Overloaded,
        _ => LlmError::Server { status, message: body },
    }
}

// ---------------------------------------------------------------------------
// LlmProvider impl
// ---------------------------------------------------------------------------

#[async_trait]
impl LlmProvider for VertexProvider {
    fn name(&self) -> &str {
        "vertex"
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![ModelInfo {
            id: self.model.clone(),
            display_name: self.model.clone(),
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
                StreamEvent::MessageDelta {
                    usage: Some(u), ..
                } => {
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
            ProviderFeature::ToolUse | ProviderFeature::Streaming => true,
            ProviderFeature::SystemPrompt => is_claude,
            ProviderFeature::Vision => true,
        }
    }
}
