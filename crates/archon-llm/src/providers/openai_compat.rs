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

        json!({
            "model": model,
            "messages": messages,
            "max_tokens": req.max_tokens,
        })
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
        let json: Value = resp
            .json()
            .await
            .map_err(|e| LlmError::Serialize(format!("invalid JSON body: {e}")))?;
        Self::parse_chat_response(json)
    }

    async fn stream(
        &self,
        _request: LlmRequest,
    ) -> Result<Receiver<StreamEvent>, LlmError> {
        Err(LlmError::Unsupported(
            "streaming deferred to TASK-AGS-707".into(),
        ))
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
