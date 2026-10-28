/// Google Vertex AI provider implementing `LlmProvider`.
///
/// Supports Claude models via the Anthropic Messages API format and
/// Gemini models via the Gemini `contents` API format.
use async_trait::async_trait;
use tokio::sync::mpsc::Receiver;

use crate::provider::{
    DataFlowClassification, LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo,
    ProviderFeature,
};
use crate::providers::gcp_auth::{GcpAccessToken, get_access_token, resolve_credentials};
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
            http: crate::transport_http::client_builder()
                .build()
                .expect("default HTTP client configuration is valid"),
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
        if let Some(ref token) = *cache
            && token.expires_at > std::time::Instant::now()
        {
            return Ok(token.access_token.clone());
        }

        // Fetch fresh token.
        let creds = resolve_credentials(self.credentials_file.as_deref())?;
        let token = get_access_token(&self.http, &creds).await?;
        let access_token = token.access_token.clone();
        *cache = Some(token);
        Ok(access_token)
    }

    /// Render the `system` field for Claude on Vertex.
    ///
    /// Vertex serves the Anthropic Messages API, so a `cache_control` marker on
    /// a system block reaches the wire untouched — but only in the *array* form.
    /// Flattening every block into one string, as this did, silently discarded
    /// the marker, which is why the system prefix was never cached here even
    /// though `cache_strategy` advertised that it was.
    ///
    /// The array is emitted only when a marker is actually present. Requests
    /// that are not caching keep the exact string body Vertex has always
    /// received, so nothing changes for existing deployments.
    pub fn build_system_field(system: &[serde_json::Value]) -> Option<serde_json::Value> {
        if system.is_empty() {
            return None;
        }

        if system.iter().any(|b| b.get("cache_control").is_some()) {
            let blocks: Vec<serde_json::Value> = system
                .iter()
                .filter(|b| {
                    b.get("text")
                        .and_then(|t| t.as_str())
                        .is_some_and(|s| !s.is_empty())
                })
                .map(|b| {
                    let mut block = b.clone();
                    // Anthropic requires the discriminant; archon's own blocks
                    // carry it, but a caller-supplied one may not.
                    if let Some(obj) = block.as_object_mut() {
                        obj.entry("type")
                            .or_insert_with(|| serde_json::Value::String("text".into()));
                    }
                    block
                })
                .collect();

            if !blocks.is_empty() {
                return Some(serde_json::Value::Array(blocks));
            }
        }

        let system_text: String = system
            .iter()
            .filter_map(|b| {
                b.get("text")
                    .and_then(|t| t.as_str())
                    .map(|s| s.to_string())
            })
            .collect::<Vec<_>>()
            .join("\n");

        if system_text.is_empty() {
            None
        } else {
            Some(serde_json::Value::String(system_text))
        }
    }

    /// Build the request body for Claude on Vertex (Anthropic Messages format).
    pub fn build_claude_body(request: &LlmRequest) -> serde_json::Value {
        let mut body = serde_json::json!({
            "anthropic_version": "vertex-2023-10-16",
            "max_tokens": request.max_tokens,
            "messages": request.messages,
            "stream": true
        });

        if let Some(system) = Self::build_system_field(&request.system) {
            body["system"] = system;
        }

        if !request.tools.is_empty() {
            body["tools"] = serde_json::Value::Array(request.tools.as_ref().clone());
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

                let parts = if let Some(content_arr) = msg.get("content").and_then(|c| c.as_array())
                {
                    content_arr
                        .iter()
                        .filter_map(|block| {
                            block
                                .get("text")
                                .and_then(|t| t.as_str())
                                .map(|text| serde_json::json!({"text": text}))
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
        crate::provider_fault::begin_request();
        let access_token = self.get_token().await?;
        self.stream_at(request, &self.endpoint_url(), &access_token)
            .await
    }

    async fn stream_at(
        &self,
        request: LlmRequest,
        url: &str,
        access_token: &str,
    ) -> Result<Receiver<StreamEvent>, LlmError> {
        let body = if self.is_claude_model() {
            Self::build_claude_body(&request)
        } else {
            Self::build_gemini_body(&request)
        };

        let resp = self
            .http
            .post(url)
            .bearer_auth(&access_token)
            .json(&body)
            .send()
            .await
            .map_err(crate::provider_fault::from_transport)?;

        let status = resp.status().as_u16();
        crate::provider_fault::note_response(
            status,
            crate::transport_http::response_retry_after(&resp),
        );
        if status >= 400 {
            return Err(crate::transport_http::response_error(resp, map_http_error).await);
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
                        let _ = tx.send(crate::provider_fault::transport_event(e)).await;
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
                        if let Some(data) = line.strip_prefix("data: ")
                            && let Ok(val) = serde_json::from_str::<serde_json::Value>(data)
                        {
                            for event in parse_anthropic_vertex_event(&val) {
                                if tx.send(event).await.is_err() {
                                    return;
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

#[path = "vertex_wire.rs"]
mod wire;
use wire::*;

// ---------------------------------------------------------------------------
// LlmProvider impl
// ---------------------------------------------------------------------------

#[async_trait]
impl LlmProvider for VertexProvider {
    fn name(&self) -> &str {
        "vertex"
    }

    /// Vertex serves Anthropic's own figures. It is still named rather than
    /// folded into `AnthropicApi`, because Google does withhold the extended TTL
    /// on several older Claude models — that simply costs nothing to express
    /// today, since none of them carry a documented one-hour TTL on any stack.
    fn cache_platform(&self) -> crate::cache_models::CachePlatform {
        crate::cache_models::CachePlatform::Vertex
    }

    /// Claude on Vertex takes Anthropic-style `cache_control`, and
    /// `build_claude_body` passes `request.messages` through verbatim, so a
    /// marker written by `apply_conversation_cache` reaches the wire as-is.
    /// `build_system_field` now does the same for system blocks, so the
    /// stable-head breakpoint survives too — it used to be flattened away.
    ///
    /// Gemini has no such field and gets nothing rather than a 400.
    fn cache_strategy(&self, model: &str) -> crate::cache_strategy::CacheStrategy {
        // One provider instance serves one deployed model; `self.model` names
        // it whatever alias the request carries.
        let _ = model;
        if !self.is_claude_model() {
            return crate::cache_strategy::CacheStrategy::None;
        }
        crate::cache_strategy::anthropic_for_model(
            &self.model,
            crate::cache_models::CachePlatform::Vertex,
        )
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![ModelInfo {
            id: self.model.clone(),
            display_name: self.model.clone(),
            context_window: 0,
        }]
    }

    fn compaction_provider_family(&self) -> crate::compaction_policy::ProviderFamily {
        crate::compaction_policy::ProviderFamily::Vertex
    }

    async fn stream(&self, mut request: LlmRequest) -> Result<Receiver<StreamEvent>, LlmError> {
        self.resolve_request_model(&mut request);
        if !self.is_claude_model() && !request.tools.is_empty() {
            return Err(LlmError::Unsupported(
                "Gemini tool use on Vertex is not implemented".into(),
            ));
        }
        request.messages = crate::message_invariants::sanitize_anthropic_shape(request.messages);
        self.do_stream(request).await
    }

    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse, LlmError> {
        let mut rx = self.stream(request).await?;
        let mut text_parts: Vec<String> = Vec::new();
        // Issue 424: delta usage is cumulative for its message, never added.
        let mut usage = crate::usage::UsageAccumulator::default();
        let mut stop_reason = String::new();

        while let Some(event) = rx.recv().await {
            usage.record_event(&event);
            match event {
                StreamEvent::TextDelta { text, .. } => {
                    text_parts.push(text);
                }
                StreamEvent::MessageDelta {
                    stop_reason: Some(sr),
                    usage: Some(_),
                } => {
                    stop_reason = sr;
                }
                StreamEvent::MessageDelta {
                    stop_reason: Some(sr),
                    ..
                } => {
                    stop_reason = sr;
                }
                StreamEvent::Error {
                    error_type,
                    message,
                } => {
                    return Err(crate::provider_fault::from_stream_error(
                        &error_type,
                        &message,
                    ));
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
            usage: usage.to_usage(),
            stop_reason,
        })
    }

    fn supports_feature(&self, feature: ProviderFeature) -> bool {
        let is_claude = self.is_claude_model();
        match feature {
            ProviderFeature::Thinking
            | ProviderFeature::PromptCaching
            | ProviderFeature::ToolUse => is_claude,
            ProviderFeature::Streaming => true,
            ProviderFeature::SystemPrompt => is_claude,
            ProviderFeature::Vision => true,
        }
    }

    fn data_flow_classification(&self) -> DataFlowClassification {
        DataFlowClassification::Cloud
    }
}

#[cfg(test)]
#[path = "vertex_tests.rs"]
mod tests;

#[cfg(test)]
mod connection_recovery_tests {
    use super::*;

    #[tokio::test]
    async fn calls_after_sleep_use_fresh_connections() {
        let client = VertexProvider::new(
            "AC-X-001".into(),
            "AC-X-001".into(),
            String::new(),
            String::new(),
            None,
        );
        crate::connection_tests::assert_fresh_connections(client.http).await;
    }
}

#[cfg(test)]
mod header_recovery_tests {
    #[tokio::test]
    async fn response_header_retains_typed_retry_after() {
        let response = crate::connection_tests::error_response(503, "90").await;
        let error = crate::transport_http::response_error(response, super::map_http_error).await;
        assert!(
            matches!(
                error,
                crate::provider::LlmError::Overloaded {
                    retry_after_secs: Some(90),
                    ..
                } | crate::provider::LlmError::Server {
                    pipeline_retry_after_secs: Some(90),
                    ..
                }
            ),
            "header lost: {error:?}"
        );
    }
}

#[cfg(test)]
#[path = "vertex_recovery_tests.rs"]
mod recovery_http_tests;
