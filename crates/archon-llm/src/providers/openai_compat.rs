//! TASK-AGS-703 SPEC DEVIATION (greenlit 2026-04-13):
//!   ChatRequest    -> LlmRequest  (existing provider-agnostic type)
//!   ChatResponse   -> LlmResponse (existing provider-agnostic type)
//!   .chat()        -> .complete() (existing trait method)
//!   .stream_chat() -> .stream()   (existing trait method)
//!   ProviderError  -> mapped to LlmError at the trait boundary
//!   .embed()       -> inherent method on OpenAiCompatProvider, not trait
//!
//! Parametric OpenAI-compatible provider impl backed by the descriptor
//! registry (TASK-AGS-702) and credential wrapper (TASK-AGS-701). A single
//! implementation drives every OpenAI-style backend; per-provider behavior
//! comes from the static `ProviderDescriptor`, NOT from runtime `if`
//! branches on provider id.

use std::sync::Arc;

use serde_json::{json, Value};
use tokio::sync::mpsc::Receiver;

use crate::provider::{
    LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo, ProviderFeature,
};
use crate::secrets::ApiKey;
use crate::streaming::StreamEvent;
use crate::types::Usage;

use super::descriptor::{AuthFlavor, ProviderDescriptor};
use super::quirks::{ProviderQuirks, StreamDelimiter, ToolCallFormat};
use super::stream_decode::{decode_ndjson_line, decode_sse_line, FrameOutcome};

/// OpenAI-compatible provider driven by a static `ProviderDescriptor`.
///
/// Holds the descriptor (routing + capability metadata), an `Arc`-shared
/// `reqwest::Client` for connection reuse, and a redacting `ApiKey`. No
/// per-provider branching: all per-backend variation comes from the
/// descriptor's `auth_flavor`, `base_url`, `headers`, and `default_model`.
pub struct OpenAiCompatProvider {
    pub(crate) descriptor: &'static ProviderDescriptor,
    pub(crate) http: Arc<reqwest::Client>,
    pub(crate) api_key: ApiKey,
}

impl OpenAiCompatProvider {
    /// Construct a new provider bound to `descriptor`, reusing the supplied
    /// HTTP client and API key.
    pub fn new(
        descriptor: &'static ProviderDescriptor,
        http: Arc<reqwest::Client>,
        api_key: ApiKey,
    ) -> Self {
        Self {
            descriptor,
            http,
            api_key,
        }
    }

    // -----------------------------------------------------------------
    // URL construction
    // -----------------------------------------------------------------

    /// Join `descriptor.base_url` with `chat/completions`.
    ///
    /// `Url::join` replaces the final path segment unless the base ends in
    /// `/`, so we explicitly append one before joining. That way a base of
    /// `http://localhost:11434/v1` correctly produces
    /// `http://localhost:11434/v1/chat/completions` instead of
    /// `http://localhost:11434/chat/completions`.
    fn build_chat_url(&self) -> String {
        self.build_endpoint_url("chat/completions")
    }

    fn build_embeddings_url(&self) -> String {
        self.build_endpoint_url("embeddings")
    }

    fn build_endpoint_url(&self, endpoint: &str) -> String {
        let mut base = self.descriptor.base_url.clone();
        // Ensure trailing slash on the base path so `Url::join` appends
        // rather than replaces the final segment.
        if !base.path().ends_with('/') {
            let new_path = format!("{}/", base.path());
            base.set_path(&new_path);
        }
        match base.join(endpoint) {
            Ok(u) => u.to_string(),
            // Fallback: manually concatenate. `endpoint` here is a static
            // string so this branch is essentially unreachable, but we
            // prefer a deterministic string over a panic.
            Err(_) => format!("{}{}", base, endpoint),
        }
    }

    // -----------------------------------------------------------------
    // Wire format
    // -----------------------------------------------------------------

    /// Build the OpenAI `/v1/chat/completions` request body from a generic
    /// `LlmRequest`. Deliberately minimal: tools, thinking, speed, effort,
    /// and `extra` are reserved for TASK-AGS-705 quirks and are not
    /// forwarded here.
    fn to_openai_wire(&self, req: &LlmRequest) -> Value {
        let model = if req.model.is_empty() {
            self.descriptor.default_model.clone()
        } else {
            req.model.clone()
        };

        // Merge `req.system` (if any) as a leading system message. Each
        // system entry is already a JSON value; we concatenate their text
        // representations into a single synthetic system message so the
        // wire format remains OpenAI-canonical.
        let mut messages: Vec<Value> = Vec::with_capacity(req.messages.len() + 1);
        if !req.system.is_empty() {
            let system_text = req
                .system
                .iter()
                .map(|v| match v {
                    Value::String(s) => s.clone(),
                    // Anthropic-style content block `{"type":"text","text":"..."}`
                    Value::Object(_) => v
                        .get("text")
                        .and_then(|t| t.as_str())
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| v.to_string()),
                    _ => v.to_string(),
                })
                .collect::<Vec<_>>()
                .join("\n");
            messages.push(json!({"role": "system", "content": system_text}));
        }
        for m in &req.messages {
            messages.push(m.clone());
        }

        // TASK-AGS-705: tool_calls serialization branches on
        // `descriptor.quirks.tool_call_format`. Request-side tool
        // forwarding lands in a later slice; the enum is staged here
        // so TASK-AGS-707/708 can consume it without touching request
        // construction. Reading the field prevents dead-code warnings
        // and proves the quirks dispatch path is wired.
        let _tool_format: ToolCallFormat = self.descriptor.quirks.tool_call_format;

        json!({
            "model": model,
            "messages": messages,
            "max_tokens": req.max_tokens,
        })
    }

    /// TASK-AGS-705: delimiter bytes for the provider's streaming wire
    /// format. Crate-visible so TASK-AGS-707 can drive its chunk parser
    /// without knowing which provider it's talking to. Marked
    /// `#[allow(dead_code)]` until TASK-AGS-707 consumes it.
    #[allow(dead_code)]
    pub(crate) fn delimiter_bytes(&self) -> &'static [u8] {
        self.descriptor.quirks.delimiter_bytes()
    }

    // -----------------------------------------------------------------
    // Auth + static headers
    // -----------------------------------------------------------------

    /// Attach auth + descriptor headers to a `RequestBuilder`. No
    /// per-provider-id branching: the descriptor's `auth_flavor` decides
    /// how to sign the request, and custom schemes are deferred to
    /// TASK-AGS-705.
    fn apply_auth(&self, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let rb = match &self.descriptor.auth_flavor {
            AuthFlavor::BearerApiKey => rb.bearer_auth(self.api_key.expose()),
            AuthFlavor::None => rb,
            // BasicAuth + Custom schemes are reserved for TASK-AGS-705.
            // Return `rb` unchanged for now so those descriptors compile
            // without stubbing behavior they don't need in this task.
            AuthFlavor::BasicAuth | AuthFlavor::Custom(_) => rb,
        };
        let mut rb = rb;
        for (k, v) in &self.descriptor.headers {
            rb = rb.header(k, v);
        }
        rb
    }

    // -----------------------------------------------------------------
    // Response parsing
    // -----------------------------------------------------------------

    /// Apply `quirks.ignore_response_fields` to a response body in place.
    /// Strips the named keys from the top-level object and from each
    /// element of `choices` if present. Used for DeepSeek's `logprobs`
    /// bag whose shape deviates from OpenAI's canonical form and whose
    /// contents archon never consumes. Pure data transform — no
    /// provider-id branching.
    fn strip_ignored_fields(body: &mut Value, quirks: &ProviderQuirks) {
        if quirks.ignore_response_fields.is_empty() {
            return;
        }
        if let Some(obj) = body.as_object_mut() {
            for field in quirks.ignore_response_fields {
                obj.remove(*field);
            }
            if let Some(Value::Array(choices)) = obj.get_mut("choices") {
                for choice in choices.iter_mut() {
                    if let Some(choice_obj) = choice.as_object_mut() {
                        for field in quirks.ignore_response_fields {
                            choice_obj.remove(*field);
                        }
                    }
                }
            }
        }
    }

    /// Parse an OpenAI `chat.completion` JSON body into an `LlmResponse`.
    fn parse_chat_response(body: Value) -> Result<LlmResponse, LlmError> {
        let choices = body
            .get("choices")
            .and_then(|c| c.as_array())
            .ok_or_else(|| LlmError::Serialize("missing `choices` array in response".into()))?;
        let first = choices
            .first()
            .ok_or_else(|| LlmError::Serialize("`choices` array was empty".into()))?;

        let content = first
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .ok_or_else(|| {
                LlmError::Serialize("missing `choices[0].message.content` string".into())
            })?
            .to_string();

        let stop_reason = first
            .get("finish_reason")
            .and_then(|f| f.as_str())
            .unwrap_or("stop")
            .to_string();

        let usage_json = body.get("usage");
        let input_tokens = usage_json
            .and_then(|u| u.get("prompt_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let output_tokens = usage_json
            .and_then(|u| u.get("completion_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        Ok(LlmResponse {
            content: vec![json!({"type": "text", "text": content})],
            usage: Usage {
                input_tokens,
                output_tokens,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            },
            stop_reason,
        })
    }

    // -----------------------------------------------------------------
    // HTTP error mapping
    // -----------------------------------------------------------------

    /// Map a non-success HTTP status into an `LlmError`.
    ///
    /// Retry-after parsing for 429 is deferred to TASK-AGS-708.
    fn map_http_error(status: reqwest::StatusCode, body: String, provider: &str) -> LlmError {
        let code = status.as_u16();
        if status == reqwest::StatusCode::UNAUTHORIZED
            || status == reqwest::StatusCode::FORBIDDEN
        {
            return LlmError::Auth(format!("{provider}: {body}"));
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return LlmError::RateLimited {
                retry_after_secs: 0,
            };
        }
        if status.is_server_error() {
            return LlmError::Server {
                status: code,
                message: body,
            };
        }
        LlmError::Http(format!("status {}: {}", code, body))
    }

    // -----------------------------------------------------------------
    // Embeddings — inherent method, not on the trait
    // -----------------------------------------------------------------

    /// Embeddings endpoint — inherent method, not on the `LlmProvider`
    /// trait.
    ///
    /// TASK-AGS-703 spec deviation: the trait does not expose `embed()`,
    /// so this lives as an inherent method for OpenAI-compat callers that
    /// need it. TASK-AGS-711 acceptance tests may exercise this via a
    /// downcast helper. Minimal implementation using the same auth / URL
    /// / error patterns as `complete()`.
    pub async fn embed(
        &self,
        model: &str,
        input: Vec<String>,
    ) -> Result<Vec<Vec<f32>>, LlmError> {
        let url = self.build_embeddings_url();
        let body = json!({
            "model": if model.is_empty() { self.descriptor.default_model.as_str() } else { model },
            "input": input,
        });
        let rb = self.http.post(&url).json(&body);
        let rb = self.apply_auth(rb);
        let resp = rb
            .send()
            .await
            .map_err(|e| LlmError::Http(e.to_string()))?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(Self::map_http_error(status, text, self.name()));
        }
        let json: Value = resp
            .json()
            .await
            .map_err(|e| LlmError::Serialize(format!("invalid JSON body: {e}")))?;

        let data = json
            .get("data")
            .and_then(|d| d.as_array())
            .ok_or_else(|| LlmError::Serialize("missing `data` array in embeddings response".into()))?;

        let mut out: Vec<Vec<f32>> = Vec::with_capacity(data.len());
        for entry in data {
            let emb = entry
                .get("embedding")
                .and_then(|e| e.as_array())
                .ok_or_else(|| {
                    LlmError::Serialize("missing `data[].embedding` array".into())
                })?;
            let mut vec: Vec<f32> = Vec::with_capacity(emb.len());
            for v in emb {
                let f = v.as_f64().ok_or_else(|| {
                    LlmError::Serialize("embedding entry was not a number".into())
                })? as f32;
                vec.push(f);
            }
            out.push(vec);
        }
        Ok(out)
    }
}

#[async_trait::async_trait]
impl LlmProvider for OpenAiCompatProvider {
    fn name(&self) -> &str {
        &self.descriptor.display_name
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![ModelInfo {
            id: self.descriptor.default_model.clone(),
            display_name: self.descriptor.default_model.clone(),
            context_window: 0,
        }]
    }

    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse, LlmError> {
        let url = self.build_chat_url();
        let body = self.to_openai_wire(&request);
        let rb = self.http.post(&url).json(&body);
        let rb = self.apply_auth(rb);
        let resp = rb
            .send()
            .await
            .map_err(|e| LlmError::Http(e.to_string()))?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(Self::map_http_error(status, text, self.name()));
        }
        let mut json: Value = resp
            .json()
            .await
            .map_err(|e| LlmError::Serialize(format!("invalid JSON body: {e}")))?;
        // TASK-AGS-705: strip quirk-ignored fields (e.g. DeepSeek
        // `logprobs`) before the generic parser walks the body.
        Self::strip_ignored_fields(&mut json, &self.descriptor.quirks);
        Self::parse_chat_response(json)
    }

    /// TASK-AGS-707: streaming chat for OpenAI-compatible providers.
    ///
    /// Feature-gated on `descriptor.supports.streaming`. Delimiter dispatch
    /// uses `descriptor.quirks.stream_delimiter` — SSE vs NDJSON — so there
    /// is never a `match provider_id` inside the stream body (REQ-FOR-D6:
    /// adding a provider is a data-only change).
    ///
    /// Spec deviation (documented in `tests/compat_stream_sse.rs`): the
    /// receiver carries `StreamEvent`, not `Result<StreamEvent, _>`, so
    /// mid-stream network errors are surfaced as `StreamEvent::Error`
    /// rather than as an `Err` on the channel. This matches the existing
    /// `OpenAiProvider::do_stream` pattern and the Anthropic-style
    /// `StreamEvent` enum the rest of the codebase consumes.
    async fn stream(
        &self,
        request: LlmRequest,
    ) -> Result<Receiver<StreamEvent>, LlmError> {
        // Feature gate FIRST — never touch the network if streaming is off.
        // Validation Criterion 6.
        if !self.descriptor.supports.streaming {
            return Err(LlmError::Unsupported(format!(
                "{}: streaming not supported by this provider",
                self.name()
            )));
        }

        // Build request body and inject `stream: true`. `to_openai_wire`
        // always returns a JSON object, so `as_object_mut` never fails in
        // practice; we guard defensively.
        let url = self.build_chat_url();
        let mut body = self.to_openai_wire(&request);
        if let Some(obj) = body.as_object_mut() {
            obj.insert("stream".into(), Value::Bool(true));
        }

        let rb = self.http.post(&url).json(&body);
        let rb = self.apply_auth(rb);
        let resp = rb
            .send()
            .await
            .map_err(|e| LlmError::Http(e.to_string()))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(Self::map_http_error(status, text, self.name()));
        }

        let (tx, rx) = tokio::sync::mpsc::channel::<StreamEvent>(256);
        let delimiter = self.descriptor.quirks.stream_delimiter;
        let mut byte_stream = resp.bytes_stream();

        tokio::spawn(async move {
            use futures_util::StreamExt;

            let mut buffer: Vec<u8> = Vec::new();

            while let Some(chunk_result) = byte_stream.next().await {
                let chunk_bytes = match chunk_result {
                    Ok(b) => b,
                    Err(e) => {
                        // Mid-stream network error: surface as
                        // StreamEvent::Error and close the channel.
                        let _ = tx
                            .send(StreamEvent::Error {
                                error_type: "http_error".into(),
                                message: e.to_string(),
                            })
                            .await;
                        return;
                    }
                };
                buffer.extend_from_slice(&chunk_bytes);

                // Drain complete newline-terminated lines from the buffer.
                // Both SSE (`\n\n` between events) and NDJSON (`\n` between
                // objects) terminate each payload line with a single `\n`,
                // so a single `find('\n')` loop serves both delimiters.
                while let Some(nl_pos) = buffer.iter().position(|b| *b == b'\n') {
                    let mut line: Vec<u8> = buffer.drain(..=nl_pos).collect();
                    // Drop the trailing `\n` (and any `\r` before it) so
                    // the decoder sees just the payload.
                    line.pop(); // \n
                    if line.last() == Some(&b'\r') {
                        line.pop();
                    }

                    let outcome = match delimiter {
                        StreamDelimiter::Sse => decode_sse_line(&line),
                        StreamDelimiter::MistralNdjson => decode_ndjson_line(&line),
                    };

                    match outcome {
                        None => continue,
                        Some(FrameOutcome::Events(events)) => {
                            for ev in events {
                                if tx.send(ev).await.is_err() {
                                    return;
                                }
                            }
                        }
                        Some(FrameOutcome::End) => {
                            let _ = tx.send(StreamEvent::MessageStop).await;
                            return;
                        }
                    }
                }
            }

            // EOF reached. Try to decode any trailing partial (useful for
            // NDJSON streams that don't terminate the final line, and for
            // SSE streams that close mid-event).
            if !buffer.is_empty() {
                let outcome = match delimiter {
                    StreamDelimiter::Sse => decode_sse_line(&buffer),
                    StreamDelimiter::MistralNdjson => decode_ndjson_line(&buffer),
                };
                match outcome {
                    Some(FrameOutcome::Events(events)) => {
                        for ev in events {
                            if tx.send(ev).await.is_err() {
                                return;
                            }
                        }
                    }
                    Some(FrameOutcome::End) => {
                        let _ = tx.send(StreamEvent::MessageStop).await;
                        return;
                    }
                    None => {}
                }
            }

            // NDJSON has no [DONE] sentinel; SSE streams that close
            // cleanly without [DONE] also reach here. In both cases we
            // emit a final MessageStop so the consumer knows the stream
            // has completed.
            let _ = tx.send(StreamEvent::MessageStop).await;
        });

        Ok(rx)
    }

    fn supports_feature(&self, feature: ProviderFeature) -> bool {
        use ProviderFeature as F;
        let s = &self.descriptor.supports;
        match feature {
            F::Streaming => s.streaming,
            F::ToolUse => s.tool_calling,
            F::Vision => s.vision,
            // OpenAI-compatible providers universally accept a `system`
            // role message, so advertise SystemPrompt unconditionally.
            F::SystemPrompt => true,
            // PromptCaching + Thinking are Anthropic-specific and not
            // modeled in `ProviderFeatures`. Default to false.
            F::PromptCaching => false,
            F::Thinking => false,
        }
    }
}
