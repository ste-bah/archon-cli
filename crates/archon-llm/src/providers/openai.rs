use async_trait::async_trait;
use tokio::sync::mpsc::Receiver;

use crate::provider::{
    DataFlowClassification, LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo,
    ProviderFeature, classify_data_flow_endpoint,
};
use crate::providers::openai_cache::{supports_explicit_prompt_cache, system_content_parts};
use crate::providers::openai_protocol::map_http_error;
use crate::streaming::StreamEvent;
use crate::types::Usage;

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
        Self::build_openai_messages_cached(system, messages, None)
    }

    /// As [`Self::build_openai_messages`], optionally closing the stable head of
    /// the system prompt with a `prompt_cache_breakpoint`.
    ///
    /// The breakpoint has to be built here because it rides on a content *part*,
    /// and this is the only place the parts exist. Without a placement the
    /// system prompt stays the single joined string every OpenAI-compatible host
    /// has always received.
    pub fn build_openai_messages_cached(
        system: &[serde_json::Value],
        messages: &[serde_json::Value],
        cache: Option<&crate::cache_wire::OpenAiCachePlacement>,
    ) -> Vec<serde_json::Value> {
        let mut result = Vec::new();

        if let Some(cache) = cache {
            let parts = system_content_parts(system, cache.stable_system_blocks);
            if !parts.is_empty() {
                result.push(serde_json::json!({
                    "role": "system",
                    "content": parts
                }));
            }
        } else {
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
        let cache = crate::cache_wire::openai_cache_placement(
            &request.extra,
            &request.system,
            &request.messages,
            &request.tools,
        );
        let body = build_openai_stream_request_body_cached(
            &request.model,
            request.max_tokens,
            &request.system,
            &request.messages,
            &request.tools,
            cache.as_ref(),
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
    build_openai_request_body_cached(model, max_tokens, system, messages, tools, stream, None)
}

/// As [`build_openai_request_body`], with an optional prompt-cache placement.
///
/// `prompt_cache_options` is sent only for `explicit` mode, because it turns
/// OpenAI's own implicit breakpoints **off**. In `hybrid` the breakpoint is
/// added alongside them, so a misjudged placement costs nothing rather than
/// costing the caching that would otherwise have happened by itself.
pub fn build_openai_request_body_cached(
    model: &str,
    max_tokens: u32,
    system: &[serde_json::Value],
    messages: &[serde_json::Value],
    tools: &[serde_json::Value],
    stream: bool,
    cache: Option<&crate::cache_wire::OpenAiCachePlacement>,
) -> serde_json::Value {
    let openai_messages = OpenAiProvider::build_openai_messages_cached(system, messages, cache);
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

    if let Some(cache) = cache {
        body["prompt_cache_key"] = serde_json::json!(cache.cache_key);
        if cache.explicit_only {
            body["prompt_cache_options"] = serde_json::json!({ "mode": "explicit" });
        }
    }

    body
}

pub fn build_openai_stream_request_body(
    model: &str,
    max_tokens: u32,
    system: &[serde_json::Value],
    messages: &[serde_json::Value],
    tools: &[serde_json::Value],
) -> serde_json::Value {
    build_openai_stream_request_body_cached(model, max_tokens, system, messages, tools, None)
}

pub fn build_openai_stream_request_body_cached(
    model: &str,
    max_tokens: u32,
    system: &[serde_json::Value],
    messages: &[serde_json::Value],
    tools: &[serde_json::Value],
    cache: Option<&crate::cache_wire::OpenAiCachePlacement>,
) -> serde_json::Value {
    let mut body =
        build_openai_request_body_cached(model, max_tokens, system, messages, tools, true, cache);
    body["stream_options"] = serde_json::json!({"include_usage": true});
    body
}

// SSE parsing lives in `openai_stream`; re-exported here so existing
// `providers::openai::parse_openai_sse_chunk` call sites keep working.
pub(crate) use super::openai_stream::parse_openai_sse_chunk;

// ---------------------------------------------------------------------------
// LlmProvider impl
// ---------------------------------------------------------------------------

#[async_trait]
impl LlmProvider for OpenAiProvider {
    fn name(&self) -> &str {
        "openai"
    }

    fn cache_platform(&self) -> crate::cache_models::CachePlatform {
        crate::cache_models::CachePlatform::OpenAiApi
    }

    /// OpenAI caches a stable prefix automatically above 1,024 tokens, with
    /// nothing to annotate — so this is [`CacheStrategy::Automatic`], not
    /// [`CacheStrategy::None`].
    ///
    /// The distinction is not cosmetic. `None` means "this endpoint does not
    /// cache", and `caches()` is false for it, so cost reporting treats every
    /// request as uncached. Requests were still being cached server-side; archon
    /// simply could not see it. `Automatic` reports the caching while still
    /// emitting no markers, which is the correct pair for this API.
    ///
    /// From GPT-5.6 the automatic behaviour is joined by an explicit one:
    /// `prompt_cache_breakpoint` on a content part, which archon does emit —
    /// see [`supports_explicit_prompt_cache`]. Older models reject
    /// `prompt_cache_options` outright, so the version gate is not optional.
    ///
    /// Gated on the endpoint too, because `base_url` is overridable and the same
    /// struct is pointed at Azure and other compatible hosts whose caching
    /// behaviour is not OpenAI's to promise. An operator who knows better can
    /// say so with `prompt_cache_strategy = "responses"`.
    fn cache_strategy(&self, model: &str) -> crate::cache_strategy::CacheStrategy {
        let base = self.base_url.trim_end_matches('/');
        if !base.starts_with("https://api.openai.com") {
            return crate::cache_strategy::CacheStrategy::None;
        }
        if supports_explicit_prompt_cache(model) {
            crate::cache_strategy::CacheStrategy::ResponsesBreakpoints {
                max: 4,
                min_tokens: 1024,
            }
        } else {
            crate::cache_strategy::CacheStrategy::Automatic
        }
    }

    fn compaction_provider_family(&self) -> crate::compaction_policy::ProviderFamily {
        crate::compaction_policy::ProviderFamily::OpenAiNative
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![
            ModelInfo {
                id: "gpt-4o".to_string(),
                display_name: "GPT-4o".to_string(),
                context_window: 0,
            },
            ModelInfo {
                id: "gpt-4o-mini".to_string(),
                display_name: "GPT-4o mini".to_string(),
                context_window: 0,
            },
            ModelInfo {
                id: "gpt-4-turbo".to_string(),
                display_name: "GPT-4 Turbo".to_string(),
                context_window: 0,
            },
            ModelInfo {
                id: "o1".to_string(),
                display_name: "o1".to_string(),
                context_window: 0,
            },
            ModelInfo {
                id: "o3-mini".to_string(),
                display_name: "o3-mini".to_string(),
                context_window: 0,
            },
        ]
    }

    async fn stream(&self, mut request: LlmRequest) -> Result<Receiver<StreamEvent>, LlmError> {
        self.resolve_request_model(&mut request);
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
                    stop_reason: event_stop_reason,
                    usage: event_usage,
                } => {
                    if let Some(sr) = event_stop_reason {
                        stop_reason = sr;
                    }
                    if let Some(event_usage) = event_usage {
                        usage.merge(&event_usage);
                    }
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

    fn data_flow_classification(&self) -> DataFlowClassification {
        classify_data_flow_endpoint(&self.base_url)
    }
}
