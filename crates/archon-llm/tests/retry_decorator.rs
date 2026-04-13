//! TASK-AGS-708 Gate 1: integration tests for `RetryProvider<P>`.
//!
//! Written BEFORE `crates/archon-llm/src/retry.rs` exists so the impl
//! has a compile-and-pass target. Pins the public behavior required by
//! ERR-PROV-02 (line 1850: "Retry with exponential backoff up to 3
//! attempts; then surface error") and TASK-AGS-708 Validation Criteria
//! 2-8.
//!
//! Phase-7 spec deviation (greenlit 2026-04-13):
//!   Spec wording references ProviderError variants (Unreachable, Http,
//!   AuthFailed, InvalidResponse, MissingCredential). TASK-AGS-703
//!   already re-mapped the `LlmProvider` trait to surface `LlmError` at
//!   the boundary, so `classify()` must operate on `LlmError`:
//!
//!       Retry    : Http, Server{5xx}, RateLimited, Overloaded
//!       FailFast : Auth, Serialize, Unsupported, Server{4xx},
//!                  ProviderNotFound

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::mpsc::Receiver;

use archon_llm::provider::{
    LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo, ProviderFeature,
};
use archon_llm::retry::{classify, RetryDecision, RetryPolicy, RetryProvider};
use archon_llm::streaming::StreamEvent;
use archon_llm::types::Usage;

// ---------------------------------------------------------------------------
// MockProvider — scripted responses with per-call counter
// ---------------------------------------------------------------------------

struct MockProvider {
    calls: AtomicU32,
    script: Mutex<VecDeque<Result<LlmResponse, LlmError>>>,
}

impl MockProvider {
    fn new(script: Vec<Result<LlmResponse, LlmError>>) -> Self {
        Self {
            calls: AtomicU32::new(0),
            script: Mutex::new(script.into_iter().collect()),
        }
    }

    fn calls(&self) -> u32 {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl LlmProvider for MockProvider {
    fn name(&self) -> &str {
        "mock"
    }

    fn models(&self) -> Vec<ModelInfo> {
        Vec::new()
    }

    fn supports_feature(&self, _feature: ProviderFeature) -> bool {
        true
    }

    async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse, LlmError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.script
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Err(LlmError::Http("script exhausted".into())))
    }

    async fn stream(
        &self,
        _request: LlmRequest,
    ) -> Result<Receiver<StreamEvent>, LlmError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        // Every test in this file drives complete(); stream() isn't exercised.
        Err(LlmError::Unsupported("mock-stream".into()))
    }
}

fn ok_response() -> LlmResponse {
    LlmResponse {
        content: Vec::new(),
        usage: Usage::default(),
        stop_reason: "stop".into(),
    }
}

fn base_request() -> LlmRequest {
    LlmRequest {
        model: "mock-model".into(),
        max_tokens: 64,
        system: Vec::new(),
        messages: vec![serde_json::json!({"role": "user", "content": "ping"})],
        tools: Vec::new(),
        thinking: None,
        speed: None,
        effort: None,
        extra: serde_json::Value::Null,
    }
}

/// Tight policy: no jitter, tiny backoffs so wall-clock tests stay fast.
fn tight_policy() -> RetryPolicy {
    RetryPolicy {
        max_attempts: 3,
        initial_backoff: Duration::from_millis(1),
        max_backoff: Duration::from_millis(2),
        multiplier: 2.0,
        jitter: false,
    }
}

// ---------------------------------------------------------------------------
// Validation Criterion 2: retry up to max_attempts on retryable errors
// ---------------------------------------------------------------------------

#[tokio::test]
async fn retries_three_times_on_http_error() {
    let inner = Arc::new(MockProvider::new(vec![
        Err(LlmError::Http("boom 1".into())),
        Err(LlmError::Http("boom 2".into())),
        Err(LlmError::Http("boom 3 (final)".into())),
    ]));
    let inner_for_count = inner.clone();
    let provider = RetryProvider::new(Arc::new(ArcDelegate(inner)), tight_policy());

    let err = provider
        .complete(base_request())
        .await
        .expect_err("all attempts must fail");

    assert_eq!(
        inner_for_count.calls(),
        3,
        "ERR-PROV-02: inner must be called exactly max_attempts=3 times (not 4)"
    );
    match err {
        LlmError::Http(msg) => assert!(msg.contains("boom 3"), "must surface last error, got {msg}"),
        other => panic!("expected LlmError::Http, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Validation Criterion 3: fail fast on persistent errors
// ---------------------------------------------------------------------------

#[tokio::test]
async fn fails_fast_on_auth_error() {
    let inner = Arc::new(MockProvider::new(vec![Err(LlmError::Auth("bad key".into()))]));
    let inner_for_count = inner.clone();
    let provider = RetryProvider::new(Arc::new(ArcDelegate(inner)), tight_policy());

    let _err = provider
        .complete(base_request())
        .await
        .expect_err("auth error must fail fast");

    assert_eq!(
        inner_for_count.calls(),
        1,
        "persistent Auth error must NOT be retried; inner must be called exactly once"
    );
}

#[tokio::test]
async fn fails_fast_on_unsupported_error() {
    let inner = Arc::new(MockProvider::new(vec![Err(LlmError::Unsupported(
        "feature-x".into(),
    ))]));
    let inner_for_count = inner.clone();
    let provider = RetryProvider::new(Arc::new(ArcDelegate(inner)), tight_policy());

    let _err = provider.complete(base_request()).await.expect_err("unsupported");
    assert_eq!(inner_for_count.calls(), 1, "Unsupported must fail fast");
}

#[tokio::test]
async fn fails_fast_on_server_4xx() {
    let inner = Arc::new(MockProvider::new(vec![Err(LlmError::Server {
        status: 400,
        message: "bad request".into(),
    })]));
    let inner_for_count = inner.clone();
    let provider = RetryProvider::new(Arc::new(ArcDelegate(inner)), tight_policy());

    let _err = provider
        .complete(base_request())
        .await
        .expect_err("4xx must fail fast");
    assert_eq!(inner_for_count.calls(), 1, "Server 4xx must fail fast");
}

// ---------------------------------------------------------------------------
// Validation Criterion 4: succeeds on second attempt
// ---------------------------------------------------------------------------

#[tokio::test]
async fn succeeds_on_second_attempt() {
    let inner = Arc::new(MockProvider::new(vec![
        Err(LlmError::Http("transient".into())),
        Ok(ok_response()),
    ]));
    let inner_for_count = inner.clone();
    let provider = RetryProvider::new(Arc::new(ArcDelegate(inner)), tight_policy());

    let resp = provider
        .complete(base_request())
        .await
        .expect("second attempt must succeed");
    assert_eq!(resp.stop_reason, "stop");
    assert_eq!(
        inner_for_count.calls(),
        2,
        "inner must be called exactly twice (fail, then ok)"
    );
}

// ---------------------------------------------------------------------------
// Validation Criterion 5: backoff progression (no jitter, paused time)
// ---------------------------------------------------------------------------
//
// Spec cites sleeps of 500ms, 1000ms, 2000ms. With max_attempts=3 total,
// the loop produces exactly TWO sleeps (between the 3 attempts). With
// max_attempts=4 (i.e. 1 initial + 3 retries) the full 500/1000/2000
// progression shows. We pin both: first the max_attempts=3 case (2 sleeps)
// then the max_attempts=4 case (3 sleeps) to prove the progression is
// `initial_backoff * multiplier^attempt` capped by max_backoff.

#[tokio::test(start_paused = true)]
async fn backoff_progression_without_jitter_three_attempts() {
    let policy = RetryPolicy {
        max_attempts: 3,
        initial_backoff: Duration::from_millis(500),
        max_backoff: Duration::from_secs(8),
        multiplier: 2.0,
        jitter: false,
    };
    let inner = Arc::new(MockProvider::new(vec![
        Err(LlmError::Http("1".into())),
        Err(LlmError::Http("2".into())),
        Err(LlmError::Http("3".into())),
    ]));
    let inner_for_count = inner.clone();
    let provider = RetryProvider::new(Arc::new(ArcDelegate(inner)), policy);

    let start = tokio::time::Instant::now();
    let _err = provider
        .complete(base_request())
        .await
        .expect_err("all attempts must fail");
    let elapsed = start.elapsed();

    assert_eq!(inner_for_count.calls(), 3);
    // Two sleeps: 500ms + 1000ms = 1500ms total.
    assert_eq!(
        elapsed,
        Duration::from_millis(1500),
        "paused-time elapsed must equal the sum of backoffs (500 + 1000)"
    );
}

#[tokio::test(start_paused = true)]
async fn backoff_progression_without_jitter_four_attempts_shows_500_1000_2000() {
    let policy = RetryPolicy {
        max_attempts: 4,
        initial_backoff: Duration::from_millis(500),
        max_backoff: Duration::from_secs(8),
        multiplier: 2.0,
        jitter: false,
    };
    let inner = Arc::new(MockProvider::new(vec![
        Err(LlmError::Http("1".into())),
        Err(LlmError::Http("2".into())),
        Err(LlmError::Http("3".into())),
        Err(LlmError::Http("4".into())),
    ]));
    let inner_for_count = inner.clone();
    let provider = RetryProvider::new(Arc::new(ArcDelegate(inner)), policy);

    let start = tokio::time::Instant::now();
    let _err = provider
        .complete(base_request())
        .await
        .expect_err("all attempts must fail");
    let elapsed = start.elapsed();

    assert_eq!(inner_for_count.calls(), 4);
    // Three sleeps: 500 + 1000 + 2000 = 3500ms total.
    assert_eq!(
        elapsed,
        Duration::from_millis(3500),
        "paused-time elapsed must equal 500+1000+2000 ms"
    );
}

#[tokio::test(start_paused = true)]
async fn backoff_clamped_to_max_backoff() {
    let policy = RetryPolicy {
        max_attempts: 4,
        initial_backoff: Duration::from_millis(500),
        max_backoff: Duration::from_millis(800), // clamp below the 1000ms step
        multiplier: 2.0,
        jitter: false,
    };
    let inner = Arc::new(MockProvider::new(vec![
        Err(LlmError::Http("1".into())),
        Err(LlmError::Http("2".into())),
        Err(LlmError::Http("3".into())),
        Err(LlmError::Http("4".into())),
    ]));
    let provider = RetryProvider::new(Arc::new(ArcDelegate(inner)), policy);

    let start = tokio::time::Instant::now();
    let _ = provider.complete(base_request()).await;
    let elapsed = start.elapsed();
    // Sleeps: min(500,800) + min(1000,800) + min(2000,800) = 500 + 800 + 800 = 2100ms.
    assert_eq!(elapsed, Duration::from_millis(2100));
}

// ---------------------------------------------------------------------------
// Validation Criterion 6: classify() decision table
// ---------------------------------------------------------------------------

#[test]
fn classify_retry_variants() {
    assert_eq!(
        classify(&LlmError::Http("x".into())),
        RetryDecision::Retry,
        "Http -> Retry"
    );
    assert_eq!(
        classify(&LlmError::RateLimited { retry_after_secs: 1 }),
        RetryDecision::Retry,
        "RateLimited -> Retry"
    );
    assert_eq!(
        classify(&LlmError::Overloaded),
        RetryDecision::Retry,
        "Overloaded -> Retry"
    );
    assert_eq!(
        classify(&LlmError::Server { status: 500, message: "x".into() }),
        RetryDecision::Retry,
        "Server 500 -> Retry"
    );
    assert_eq!(
        classify(&LlmError::Server { status: 503, message: "x".into() }),
        RetryDecision::Retry,
        "Server 503 -> Retry"
    );
}

#[test]
fn classify_fail_fast_variants() {
    assert_eq!(
        classify(&LlmError::Auth("x".into())),
        RetryDecision::FailFast,
        "Auth -> FailFast"
    );
    assert_eq!(
        classify(&LlmError::Serialize("x".into())),
        RetryDecision::FailFast,
        "Serialize -> FailFast"
    );
    assert_eq!(
        classify(&LlmError::Unsupported("x".into())),
        RetryDecision::FailFast,
        "Unsupported -> FailFast"
    );
    assert_eq!(
        classify(&LlmError::Server { status: 400, message: "x".into() }),
        RetryDecision::FailFast,
        "Server 4xx -> FailFast"
    );
    assert_eq!(
        classify(&LlmError::Server { status: 404, message: "x".into() }),
        RetryDecision::FailFast,
        "Server 404 -> FailFast"
    );
    assert_eq!(
        classify(&LlmError::ProviderNotFound {
            name: "x".into(),
            available: "y".into()
        }),
        RetryDecision::FailFast,
        "ProviderNotFound -> FailFast"
    );
}

// ---------------------------------------------------------------------------
// Validation Criterion 8: RetryPolicy::default().max_attempts == 3
// ---------------------------------------------------------------------------

#[test]
fn default_policy_matches_err_prov_02() {
    let p = RetryPolicy::default();
    assert_eq!(
        p.max_attempts, 3,
        "ERR-PROV-02 (line 1850): default must be 3 attempts"
    );
    assert_eq!(p.initial_backoff, Duration::from_millis(500));
    assert_eq!(p.max_backoff, Duration::from_secs(8));
    assert_eq!(p.multiplier, 2.0);
    assert!(p.jitter, "default policy must apply jitter to avoid thundering herd");
}

// ---------------------------------------------------------------------------
// Bonus: RateLimited honors retry_after_secs (NFR-RELIABILITY-003)
// ---------------------------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn rate_limited_sleeps_for_retry_after() {
    let policy = RetryPolicy {
        max_attempts: 2,
        initial_backoff: Duration::from_millis(10), // would be much smaller if honored
        max_backoff: Duration::from_secs(8),
        multiplier: 2.0,
        jitter: false,
    };
    let inner = Arc::new(MockProvider::new(vec![
        Err(LlmError::RateLimited { retry_after_secs: 2 }),
        Ok(ok_response()),
    ]));
    let provider = RetryProvider::new(Arc::new(ArcDelegate(inner)), policy);

    let start = tokio::time::Instant::now();
    let resp = provider.complete(base_request()).await.expect("retry succeeds");
    let elapsed = start.elapsed();
    assert_eq!(resp.stop_reason, "stop");
    assert_eq!(
        elapsed,
        Duration::from_secs(2),
        "RateLimited must honor retry_after_secs instead of backoff formula"
    );
}

// ---------------------------------------------------------------------------
// Helper: wrap Arc<MockProvider> in a newtype implementing LlmProvider by
// delegation so the tests can observe call counts via the Arc after moving
// the provider into RetryProvider.
// ---------------------------------------------------------------------------

struct ArcDelegate(Arc<MockProvider>);

#[async_trait]
impl LlmProvider for ArcDelegate {
    fn name(&self) -> &str {
        self.0.name()
    }
    fn models(&self) -> Vec<ModelInfo> {
        self.0.models()
    }
    fn supports_feature(&self, f: ProviderFeature) -> bool {
        self.0.supports_feature(f)
    }
    async fn complete(&self, request: LlmRequest) -> Result<LlmResponse, LlmError> {
        self.0.complete(request).await
    }
    async fn stream(
        &self,
        request: LlmRequest,
    ) -> Result<Receiver<StreamEvent>, LlmError> {
        self.0.stream(request).await
    }
}
