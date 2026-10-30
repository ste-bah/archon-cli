//! TASK-AGS-708: `RetryProvider<P>` decorator applying exponential backoff
//! (with optional ±25% jitter) to retryable `LlmError` variants.
//!
//! Spec refs:
//!   - 01-functional-spec.md ERR-PROV-02 (line 1850)
//!     "Retry with exponential backoff up to 3 attempts; then surface error"
//!   - 02-technical-spec.md TECH-AGS-NFR (line 1338 "retry"),
//!     NFR-RELIABILITY-003 (auto-retry transient errors)
//!
//! Phase-7 spec deviation (greenlit 2026-04-13):
//!   Spec wording enumerates `ProviderError` variants (Unreachable / Http /
//!   AuthFailed / InvalidResponse / MissingCredential). TASK-AGS-703
//!   re-mapped the `LlmProvider` trait to surface `LlmError` at the
//!   boundary, so `classify()` and the retry loop operate on `LlmError`.
//!   Semantics preserved:
//!
//!   ```text
//!       Retry    : Http, Server { status: 5xx }, RateLimited, Overloaded
//!       FailFast : Auth, Serialize, Unsupported, Server { status: 4xx },
//!                  ProviderNotFound
//!   ```
//!
//!   Short `LlmError::RateLimited { retry_after_secs, .. }` values override the
//!   backoff formula. Very long retry windows fail fast so the caller can
//!   surface a visible/cancellable status instead of freezing a turn.
//!
//! The decision table itself — `RetryPolicy`, `RetryDecision`, `classify`, and
//! the mid-stream/reason-code classifiers — lives in the [`policy`] child
//! module; this file is the loop that acts on it.

mod policy;

use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::mpsc::Receiver;

use crate::anthropic::AnthropicClient;
use crate::provider::{
    DataFlowClassification, LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo,
    ProviderFeature,
};
use crate::runtime::{
    ProviderRuntimeEvent, ProviderRuntimeEventType, ProviderRuntimeSeverity,
    ProviderRuntimeSupervisor,
};
use crate::streaming::StreamEvent;

pub use policy::{RetryDecision, RetryPolicy, classify};

/// Preserve the existing provider-event classification when a durable caller
/// consumes the stream directly instead of peeking in this decorator.
pub fn classify_stream_failure(
    error_type: &str,
    message: impl Into<crate::provider::DiagnosticMessage>,
) -> LlmError {
    crate::provider_fault::from_stream_error(error_type, message)
}

use policy::reason_code_for_error;
pub(crate) use policy::stream_error_before_content;

/// Whether a streaming provider error should be retried by the caller's
/// no-progress policy. Rejections remain fail-fast; transient and unknown
/// provider error types use the shared retry classification.
pub fn stream_error_is_retryable(
    error_type: &str,
    message: impl Into<crate::provider::DiagnosticMessage>,
) -> bool {
    let message = message.into();
    match error_type {
        // Provider SSE errors whose semantics explicitly describe a transient
        // failure. Other known events stay on the runner's existing route:
        // HTTP/status failures propagate and rate limits enter compaction.
        "parse_error"
        | "timeout_error"
        | "api_error"
        | "overloaded_error"
        // These are transport/framing markers emitted by the provider
        // adapters; the runner has always handled them in its no-progress
        // resend window, independently of SSE error classification.
        | "network"
        | "http_error" => true,
        // Preserve the runner's original protocol recovery condition exactly.
        "protocol" => message.is_incomplete_stream(),
        // transport_idle keeps its dedicated stall handling in the runner.
        "transport_idle" => false,
        "server_error"
        | "rate_limited"
        | "rate_limit_error"
        | "authentication_error"
        | "invalid_api_key"
        | "billing_error"
        | "insufficient_quota"
        | "invalid_request_error"
        | "permission_error"
        | "not_found_error"
        | "request_too_large" => false,
        // Unknown SSE error names have no established permanent meaning.
        _ => true,
    }
}

/// Outcome of peeking at a freshly opened stream.
enum StreamProbe {
    /// The stream produced content (or ended cleanly); hand it to the caller
    /// with the peeked events put back in front.
    Usable(Receiver<StreamEvent>),
    /// The stream errored before any content existed, so replaying the request
    /// duplicates nothing.
    FailedBeforeContent {
        error_type: crate::provider::DiagnosticMessage,
        message: crate::provider::DiagnosticMessage,
    },
}

/// Decorator that retries retryable `LlmError`s according to a `RetryPolicy`.
///
/// `P: LlmProvider + ?Sized` is the wrapped provider held behind an `Arc`,
/// which lets `RetryProvider` wrap both concrete providers
/// (`RetryProvider<OpenAiCompatProvider>`) and trait objects
/// (`RetryProvider<dyn LlmProvider>`). The decorator itself implements
/// `LlmProvider` so it can be stored as `Arc<dyn LlmProvider>` and is
/// transparent to call sites.
pub struct RetryProvider<P: LlmProvider + ?Sized> {
    inner: Arc<P>,
    policy: RetryPolicy,
    supervisor: Option<Arc<Mutex<ProviderRuntimeSupervisor>>>,
}

#[async_trait]
impl<P: LlmProvider + ?Sized> LlmProvider for RetryProvider<P> {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn models(&self) -> Vec<ModelInfo> {
        self.inner.models()
    }

    fn request_identity(&self, request: &LlmRequest) -> Option<String> {
        self.inner.request_identity(request)
    }

    fn supports_temperature(&self) -> bool {
        self.inner.supports_temperature()
    }

    fn supports_feature(&self, feature: ProviderFeature) -> bool {
        self.inner.supports_feature(feature)
    }

    fn data_flow_classification(&self) -> DataFlowClassification {
        self.inner.data_flow_classification()
    }

    fn compaction_provider_family(&self) -> crate::compaction_policy::ProviderFamily {
        self.inner.compaction_provider_family()
    }

    fn compaction_policy(&self) -> crate::compaction_policy::CompactionPolicy {
        self.inner.compaction_policy()
    }

    fn resolve_alias(&self, alias: &str) -> Option<String> {
        self.inner.resolve_alias(alias)
    }

    fn resolve_request_model(&self, request: &mut LlmRequest) {
        self.inner.resolve_request_model(request)
    }

    fn cache_strategy(&self, model: &str) -> crate::cache_strategy::CacheStrategy {
        self.inner.cache_strategy(model)
    }

    fn cache_platform(&self) -> crate::cache_models::CachePlatform {
        self.inner.cache_platform()
    }

    fn as_anthropic(&self) -> Option<&AnthropicClient> {
        self.inner.as_anthropic()
    }

    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse, LlmError> {
        self.record_runtime_event(
            &request,
            ProviderRuntimeEventType::RequestStarted,
            ProviderRuntimeSeverity::Debug,
            None,
            None,
        );
        let max = self.policy.max_attempts.max(1);
        let mut attempt: u32 = 0;
        loop {
            match self.inner.complete(request.clone()).await {
                Ok(resp) => {
                    self.record_runtime_event(
                        &request,
                        ProviderRuntimeEventType::RequestSucceeded,
                        ProviderRuntimeSeverity::Info,
                        None,
                        Some(attempt),
                    );
                    return Ok(resp);
                }
                Err(err) => {
                    attempt += 1;
                    if crate::provider_fault::retry_is_deferred()
                        || attempt >= max
                        || classify(&err) == RetryDecision::FailFast
                    {
                        let reason = reason_code_for_error(&err);
                        self.record_runtime_event(
                            &request,
                            ProviderRuntimeEventType::RequestFailed,
                            ProviderRuntimeSeverity::Error,
                            Some(reason),
                            Some(attempt),
                        );
                        return Err(err);
                    }
                    let reason = reason_code_for_error(&err);
                    self.record_runtime_event(
                        &request,
                        ProviderRuntimeEventType::RequestRetry,
                        ProviderRuntimeSeverity::Warn,
                        Some(reason),
                        Some(attempt),
                    );
                    let sleep = self.sleep_for_error(&err, attempt - 1);
                    tokio::time::sleep(sleep).await;
                }
            }
        }
    }

    /// The runner owns streaming retries through its no-progress window.
    /// This wrapper only classifies pre-content rejection events and forwards
    /// transient errors so the runner can resend them under that window.
    async fn stream(&self, request: LlmRequest) -> Result<Receiver<StreamEvent>, LlmError> {
        self.record_runtime_event(
            &request,
            ProviderRuntimeEventType::RequestStarted,
            ProviderRuntimeSeverity::Debug,
            None,
            None,
        );
        match self.inner.stream(request.clone()).await {
            Ok(rx) => {
                // A successful open ends provider-attempt accounting. Waiting
                // for first content here would hide an opened but stalled
                // stream from the caller's no-progress clocks.
                if crate::provider_fault::retry_is_deferred() {
                    self.record_runtime_event(
                        &request,
                        ProviderRuntimeEventType::RequestSucceeded,
                        ProviderRuntimeSeverity::Info,
                        None,
                        Some(0),
                    );
                    return Ok(rx);
                }
                // Peek only to classify pre-content provider rejection. A
                // transient error is replayed as an event for the runner's
                // no-progress window to own the resend decision.
                match Self::drain_until_content_or_error(rx).await {
                    StreamProbe::Usable(rx) => {
                        self.record_runtime_event(
                            &request,
                            ProviderRuntimeEventType::RequestSucceeded,
                            ProviderRuntimeSeverity::Info,
                            None,
                            Some(0),
                        );
                        return Ok(rx);
                    }
                    StreamProbe::FailedBeforeContent {
                        error_type,
                        message,
                    } => {
                        // Streaming retries belong to the runner's existing
                        // no-progress window. Classify rejections here, but
                        // replay transient provider errors for the runner.
                        let error = stream_error_before_content(&error_type, &message);
                        if classify(&error) == RetryDecision::FailFast {
                            self.record_runtime_event(
                                &request,
                                ProviderRuntimeEventType::RequestFailed,
                                ProviderRuntimeSeverity::Error,
                                Some("stream_failed_before_content"),
                                Some(0),
                            );
                            return Err(error);
                        }
                        let (tx, rx) = tokio::sync::mpsc::channel(1);
                        let message = crate::provider::DiagnosticMessage::redacted_at_source(
                            message.as_str(),
                        );
                        let _ = tx
                            .send(StreamEvent::Error {
                                error_type,
                                message,
                            })
                            .await;
                        self.record_runtime_event(
                            &request,
                            ProviderRuntimeEventType::RequestFailed,
                            ProviderRuntimeSeverity::Warn,
                            Some("stream_failed_before_content"),
                            Some(0),
                        );
                        return Ok(rx);
                    }
                }
            }
            Err(LlmError::Rejected {
                error_type,
                message,
            }) => Err(LlmError::Rejected {
                error_type,
                message: crate::provider::DiagnosticMessage::redacted_at_source(message.as_str()),
            }),
            Err(err) => Err(err),
        }
    }
}

#[path = "retry_helpers.rs"]
mod helpers;

#[cfg(test)]
#[path = "retry/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "retry/metadata_retry_tests.rs"]
mod metadata_retry_tests;
