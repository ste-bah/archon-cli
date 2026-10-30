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

use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::mpsc::Receiver;

use crate::provider::{
    DataFlowClassification, LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo,
    ProviderFeature, classify_data_flow_endpoint,
};
use crate::secrets::ApiKey;
use crate::streaming::StreamEvent;
use crate::types::Usage;

use super::descriptor::{AuthFlavor, ProviderDescriptor};
use super::quirks::{ProviderQuirks, StreamDelimiter, ToolCallFormat};
use super::stream_decode::{FrameOutcome, decode_ndjson_line, decode_sse_line};

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

#[path = "openai_compat_request.rs"]
mod request_impl;

#[async_trait::async_trait]
impl LlmProvider for OpenAiCompatProvider {
    fn name(&self) -> &str {
        &self.descriptor.display_name
    }

    /// Which of the 31 OpenAI-compatible endpoints actually cache.
    ///
    /// Answered per provider rather than by blanket assumption, because this one
    /// struct serves everything from DeepSeek to a laptop running Ollama. The
    /// wrong answer is costly in both directions: claiming caching where there
    /// is none makes a cost report understate spend, and claiming none where
    /// there is some — the state this was in — makes archon report every request
    /// as uncached.
    ///
    /// DeepSeek is the one that mattered. `archon_core::cost` already prices its
    /// cache hits at 0.003625/Mtok against 0.435 for a miss, so archon was
    /// *billing* DeepSeek as a caching provider while its provider reported
    /// `CacheStrategy::None`. Those two cannot both be right.
    ///
    /// All of these cache automatically on a stable prefix with nothing to
    /// annotate, hence `Automatic`; none of them accept breakpoint markers.
    fn cache_strategy(&self, model: &str) -> crate::cache_strategy::CacheStrategy {
        let _ = model;
        match self.descriptor.id.as_str() {
            // Documented automatic prefix caching, billed at a reduced rate.
            "deepseek" | "zai" | "zhipu" | "qwen" | "moonshot" | "siliconflow" => {
                crate::cache_strategy::CacheStrategy::Automatic
            }
            // A router, not a service: whether a request is cached depends on
            // which upstream it lands on, which archon cannot see. Reporting
            // "unknown" is honest; claiming either answer is not.
            "openrouter" => crate::cache_strategy::CacheStrategy::None,
            // Everything else — local runtimes, plain inference hosts — has no
            // documented prompt cache.
            _ => crate::cache_strategy::CacheStrategy::None,
        }
    }

    fn cache_platform(&self) -> crate::cache_models::CachePlatform {
        // Not OpenAI's stack, whatever the wire format. `Unknown` takes the
        // strictest per-model figures, which is right for an endpoint whose
        // backing service archon cannot identify.
        crate::cache_models::CachePlatform::Unknown
    }

    fn compaction_provider_family(&self) -> crate::compaction_policy::ProviderFamily {
        crate::compaction_policy::ProviderFamily::OpenAiCompatible
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![ModelInfo {
            id: self.descriptor.default_model.clone(),
            display_name: self.descriptor.default_model.clone(),
            context_window: 0,
        }]
    }

    async fn complete(&self, mut request: LlmRequest) -> Result<LlmResponse, LlmError> {
        self.resolve_request_model(&mut request);
        if !request.tools.is_empty() && !self.supports_feature(ProviderFeature::ToolUse) {
            return Err(LlmError::Unsupported(
                format!(
                    "{} does not advertise tool use; refusing to drop {} tool definition(s)",
                    self.name(),
                    request.tools.len()
                )
                .into(),
            ));
        }
        let url = self.build_chat_url();
        let body = self.to_openai_wire(&request);
        let rb = self.http.post(&url).json(&body);
        let rb = self.apply_auth(rb);
        crate::provider_fault::begin_request();
        let resp = rb
            .send()
            .await
            .map_err(crate::provider_fault::from_transport)?;
        let status = resp.status();
        crate::provider_fault::note_response(
            status.as_u16(),
            crate::transport_http::response_retry_after(&resp),
        );
        if !status.is_success() {
            let headers = resp.headers().clone();
            crate::provider_fault::note_response(
                status.as_u16(),
                crate::transport_http::response_retry_after(&resp),
            );
            let text = resp.text().await.map_err(|error| {
                crate::provider_fault::body_read_error(status.as_u16(), &headers, error)
            })?;
            return Err(Self::map_http_error(status, &headers, text, self.name()));
        }
        let mut json: Value = resp
            .json()
            .await
            .map_err(crate::provider_fault::from_transport)?;
        // TASK-AGS-705: strip quirk-ignored fields (e.g. DeepSeek
        // `logprobs`) before the generic parser walks the body.
        Self::strip_ignored_fields(&mut json, &self.descriptor.quirks);
        Self::parse_chat_response(json)
    }

    /// Feature-gated on `descriptor.supports.streaming`. Delimiter dispatch
    /// uses `descriptor.quirks.stream_delimiter` — SSE vs NDJSON — so there
    /// is never a `match provider_id` inside the stream body (REQ-FOR-D6:
    /// adding a provider is a data-only change).
    /// Spec deviation (documented in `tests/compat_stream_sse.rs`): the
    /// receiver carries `StreamEvent`, not `Result<StreamEvent, _>`, so
    /// mid-stream network errors are surfaced as `StreamEvent::Error`
    /// rather than as an `Err` on the channel. This matches the existing
    /// `OpenAiProvider::do_stream` pattern and the Anthropic-style
    /// `StreamEvent` enum the rest of the codebase consumes.
    async fn stream(&self, mut request: LlmRequest) -> Result<Receiver<StreamEvent>, LlmError> {
        self.resolve_request_model(&mut request);
        if !request.tools.is_empty() && !self.supports_feature(ProviderFeature::ToolUse) {
            return Err(LlmError::Unsupported(
                format!(
                    "{} does not advertise tool use; refusing to drop {} tool definition(s)",
                    self.name(),
                    request.tools.len()
                )
                .into(),
            ));
        }
        // Feature gate FIRST — never touch the network if streaming is off.
        // Validation Criterion 6.
        if !self.descriptor.supports.streaming {
            return Err(LlmError::Unsupported(
                format!("{}: streaming not supported by this provider", self.name()).into(),
            ));
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
        crate::provider_fault::begin_request();
        let resp = rb
            .send()
            .await
            .map_err(crate::provider_fault::from_transport)?;

        let status = resp.status();
        crate::provider_fault::note_response(
            status.as_u16(),
            crate::transport_http::response_retry_after(&resp),
        );
        if !status.is_success() {
            let headers = resp.headers().clone();
            crate::provider_fault::note_response(
                status.as_u16(),
                crate::transport_http::response_retry_after(&resp),
            );
            let text = resp.text().await.map_err(|error| {
                crate::provider_fault::body_read_error(status.as_u16(), &headers, error)
            })?;
            return Err(Self::map_http_error(status, &headers, text, self.name()));
        }

        let (tx, rx) = tokio::sync::mpsc::channel::<StreamEvent>(256);
        let delimiter = self.descriptor.quirks.stream_delimiter;
        let mut byte_stream = resp.bytes_stream();

        tokio::spawn(async move {
            use futures_util::StreamExt;

            let mut buffer: Vec<u8> = Vec::new();
            let mut finished = false;

            while let Some(chunk_result) = byte_stream.next().await {
                let chunk_bytes = match chunk_result {
                    Ok(b) => b,
                    Err(e) => {
                        // Mid-stream network error: surface as
                        // StreamEvent::Error and close the channel.
                        let _ = tx.send(crate::provider_fault::transport_event(e)).await;
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
                                finished |= matches!(
                                    &ev,
                                    StreamEvent::MessageDelta {
                                        stop_reason: Some(_),
                                        ..
                                    }
                                );
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

            // Decode an unterminated last line before classifying EOF.
            if !buffer.is_empty() {
                let outcome = match delimiter {
                    StreamDelimiter::Sse => decode_sse_line(&buffer),
                    StreamDelimiter::MistralNdjson => decode_ndjson_line(&buffer),
                };
                match outcome {
                    Some(FrameOutcome::Events(events)) => {
                        for ev in events {
                            finished |= matches!(
                                &ev,
                                StreamEvent::MessageDelta {
                                    stop_reason: Some(_),
                                    ..
                                }
                            );
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

            // Only NDJSON defines EOF as completion; SSE requires a finish marker.
            let terminal = if finished || matches!(delimiter, StreamDelimiter::MistralNdjson) {
                StreamEvent::MessageStop
            } else {
                StreamEvent::Error {
                    error_type: "protocol".into(),
                    message: "stream ended before message_stop".into(),
                }
            };
            let _ = tx.send(terminal).await;
        });

        Ok(rx)
    }

    fn supports_temperature(&self) -> bool {
        true
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

    fn data_flow_classification(&self) -> DataFlowClassification {
        classify_data_flow_endpoint(self.descriptor.base_url.as_str())
    }
}

#[path = "openai_compat_wire.rs"]
mod wire;

#[cfg(test)]
#[path = "../compat_retry_tests.rs"]
mod retry_after_tests;
