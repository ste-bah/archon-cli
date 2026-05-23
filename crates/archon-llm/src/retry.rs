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
//!   Short `LlmError::RateLimited { retry_after_secs }` values override the
//!   backoff formula. Very long retry windows fail fast so the caller can
//!   surface a visible/cancellable status instead of freezing a turn.

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

const MAX_INLINE_RATE_LIMIT_RETRY_SECS: u64 = 60;

/// Configuration for `RetryProvider`'s backoff loop.
///
/// `max_attempts` is the *total* number of calls to `inner` per request,
/// including the first. The default of `3` matches ERR-PROV-02.
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
    pub multiplier: f64,
    pub jitter: bool,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_backoff: Duration::from_millis(500),
            max_backoff: Duration::from_secs(8),
            multiplier: 2.0,
            jitter: true,
        }
    }
}

/// Decision table for a single `LlmError`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryDecision {
    Retry,
    FailFast,
}

/// Classify an `LlmError` as retryable or persistent.
///
/// See the module docstring for the full mapping rationale.
pub fn classify(err: &LlmError) -> RetryDecision {
    match err {
        LlmError::Http(_) => RetryDecision::Retry,
        LlmError::RateLimited { retry_after_secs }
            if *retry_after_secs <= MAX_INLINE_RATE_LIMIT_RETRY_SECS =>
        {
            RetryDecision::Retry
        }
        LlmError::RateLimited { .. } => RetryDecision::FailFast,
        LlmError::Overloaded => RetryDecision::Retry,
        LlmError::Server { status, .. } if *status >= 500 => RetryDecision::Retry,

        LlmError::Auth(_)
        | LlmError::QuotaExceeded(_)
        | LlmError::Aborted
        | LlmError::Serialize(_)
        | LlmError::Unsupported(_)
        | LlmError::ContextWindowExceeded { .. }
        | LlmError::Server { .. }
        | LlmError::ProviderNotFound { .. } => RetryDecision::FailFast,
    }
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

impl<P: LlmProvider + ?Sized> RetryProvider<P> {
    pub fn new(inner: Arc<P>, policy: RetryPolicy) -> Self {
        Self {
            inner,
            policy,
            supervisor: None,
        }
    }

    pub fn new_with_supervisor(
        inner: Arc<P>,
        policy: RetryPolicy,
        supervisor: Arc<Mutex<ProviderRuntimeSupervisor>>,
    ) -> Self {
        Self {
            inner,
            policy,
            supervisor: Some(supervisor),
        }
    }

    /// Expose the policy for telemetry/introspection.
    pub fn policy(&self) -> &RetryPolicy {
        &self.policy
    }

    /// Expose the wrapped provider.
    pub fn inner(&self) -> &Arc<P> {
        &self.inner
    }

    pub fn supervisor(&self) -> Option<Arc<Mutex<ProviderRuntimeSupervisor>>> {
        self.supervisor.as_ref().map(Arc::clone)
    }

    /// Compute the sleep duration for retry `attempt` (0-indexed), honoring
    /// `max_backoff` clamp and optional ±25% jitter.
    fn backoff_for_attempt(&self, attempt: u32) -> Duration {
        let initial_ms = self.policy.initial_backoff.as_millis() as f64;
        let raw = initial_ms * self.policy.multiplier.powi(attempt as i32);
        let cap = self.policy.max_backoff.as_millis() as f64;
        let clamped = raw.min(cap);
        let final_ms = if self.policy.jitter {
            // ±25% jitter: uniform in [0.75, 1.25).
            let j = rand::random::<f64>() * 0.5 + 0.75;
            clamped * j
        } else {
            clamped
        };
        Duration::from_millis(final_ms.round() as u64)
    }

    /// Determine how long to sleep after the given error on retry `attempt`.
    /// `LlmError::RateLimited` overrides the formula with the server hint.
    fn sleep_for_error(&self, err: &LlmError, attempt: u32) -> Duration {
        if let LlmError::RateLimited { retry_after_secs } = err {
            return Duration::from_secs(*retry_after_secs);
        }
        self.backoff_for_attempt(attempt)
    }

    fn record_runtime_event(
        &self,
        request: &LlmRequest,
        event_type: ProviderRuntimeEventType,
        severity: ProviderRuntimeSeverity,
        reason_code: Option<&str>,
        retry_count: Option<u32>,
    ) {
        let Some(supervisor) = &self.supervisor else {
            return;
        };
        let mut event = ProviderRuntimeEvent::new(
            self.inner.name().to_string(),
            request
                .request_origin
                .clone()
                .unwrap_or_else(|| "provider_builder".to_string()),
            event_type,
            severity,
        )
        .with_model(request.model.clone());
        if let Some(reason) = reason_code {
            event = event.with_reason(reason);
        }
        if let Some(count) = retry_count {
            event = event.with_retry_count(count);
        }
        if let Ok(mut guard) = supervisor.lock() {
            let _ = guard.record_event(event);
        }
    }
}

fn reason_code_for_error(err: &LlmError) -> &'static str {
    match err {
        LlmError::Http(_) => "http",
        LlmError::Auth(_) => "auth",
        LlmError::RateLimited { .. } => "rate_limited",
        LlmError::Overloaded => "overloaded",
        LlmError::Server { .. } => "server",
        LlmError::Serialize(_) => "serialize",
        LlmError::Unsupported(_) => "unsupported",
        LlmError::ProviderNotFound { .. } => "provider_not_found",
        LlmError::QuotaExceeded(_) => "quota_exceeded",
        LlmError::Aborted => "aborted",
        LlmError::ContextWindowExceeded { .. } => "context_window_exceeded",
    }
}

#[async_trait]
impl<P: LlmProvider + ?Sized> LlmProvider for RetryProvider<P> {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn models(&self) -> Vec<ModelInfo> {
        self.inner.models()
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
                    if attempt >= max || classify(&err) == RetryDecision::FailFast {
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

    /// `stream()` retries only the pre-flight (the `Result` returned by
    /// the inner provider). Once the `Receiver<StreamEvent>` is open, any
    /// mid-stream failure is delivered as `StreamEvent::Error` and is out
    /// of scope for this decorator (see TASK-AGS-707 notes).
    async fn stream(&self, request: LlmRequest) -> Result<Receiver<StreamEvent>, LlmError> {
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
            match self.inner.stream(request.clone()).await {
                Ok(rx) => {
                    self.record_runtime_event(
                        &request,
                        ProviderRuntimeEventType::RequestSucceeded,
                        ProviderRuntimeSeverity::Info,
                        None,
                        Some(attempt),
                    );
                    return Ok(rx);
                }
                Err(err) => {
                    attempt += 1;
                    if attempt >= max || classify(&err) == RetryDecision::FailFast {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_rate_limit_is_retryable() {
        let err = LlmError::RateLimited {
            retry_after_secs: 30,
        };

        assert_eq!(classify(&err), RetryDecision::Retry);
    }

    #[test]
    fn long_rate_limit_is_fail_fast() {
        let err = LlmError::RateLimited {
            retry_after_secs: 8_004,
        };

        assert_eq!(classify(&err), RetryDecision::FailFast);
    }
}
