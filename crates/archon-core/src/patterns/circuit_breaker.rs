//! TASK-AGS-506: CircuitBreaker + default timeout decorator.
//!
//! Wraps any `Pattern` with circuit-breaker logic (ERR-PAT-01) and a
//! configurable timeout (REQ-ARCH-006, TC-PAT-04).
//!
//! State machine:
//!   Closed -> (failures >= threshold) -> Open
//!   Open   -> (reset_after elapsed)   -> HalfOpen
//!   HalfOpen -> (probe succeeds)      -> Closed
//!   HalfOpen -> (probe fails)         -> Open

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex;
use tokio::time::Instant;

use super::{CircuitBreakerConfig, Pattern, PatternCtx, PatternError, PatternKind};

// ---------------------------------------------------------------------------
// BreakerState
// ---------------------------------------------------------------------------

/// Internal state of the circuit breaker.
#[derive(Debug)]
pub enum BreakerState {
    /// Normal operation — all calls pass through.
    Closed,
    /// Breaker has tripped — all calls are rejected immediately.
    Open { opened_at: Instant },
    /// Trial period — a limited number of probe calls are allowed.
    HalfOpen { probes_remaining: u32 },
}

// ---------------------------------------------------------------------------
// CircuitBreaker
// ---------------------------------------------------------------------------

/// Decorator that wraps a `Pattern` with circuit-breaker + timeout behavior.
///
/// * Tracks consecutive failures and trips to `Open` after
///   `cfg.failure_threshold` consecutive errors.
/// * In `Open` state, rejects calls immediately with
///   `PatternError::CircuitOpen`.
/// * After `cfg.reset_after` elapses, transitions to `HalfOpen` and
///   allows `cfg.half_open_probes` trial calls.
/// * Wraps the inner `execute` in `tokio::time::timeout` with the
///   configured duration.
pub struct CircuitBreaker {
    /// Human-readable name (surfaced in error messages).
    pub name: String,
    /// The wrapped pattern.
    inner: Arc<dyn Pattern>,
    /// Circuit breaker configuration (thresholds, timings).
    cfg: CircuitBreakerConfig,
    /// Current breaker state (protected by async Mutex).
    state: Mutex<BreakerState>,
    /// Consecutive failure counter (atomically updated).
    consecutive_failures: AtomicU32,
    /// Timeout applied to every inner `execute` call.
    timeout: Duration,
}

impl CircuitBreaker {
    /// Create a new `CircuitBreaker` wrapping `inner`.
    pub fn wrap(
        name: impl Into<String>,
        inner: Arc<dyn Pattern>,
        cfg: CircuitBreakerConfig,
        timeout: Duration,
    ) -> Arc<Self> {
        Arc::new(Self {
            name: name.into(),
            inner,
            cfg,
            state: Mutex::new(BreakerState::Closed),
            consecutive_failures: AtomicU32::new(0),
            timeout,
        })
    }

    /// Manually reset the breaker to `Closed` (ERR-PAT-01 recovery).
    pub async fn reset(&self) {
        let mut state = self.state.lock().await;
        *state = BreakerState::Closed;
        self.consecutive_failures.store(0, Ordering::SeqCst);
    }

    // -- internal helpers ---------------------------------------------------

    /// Record a successful call: reset failures and transition to Closed.
    async fn record_success(&self) {
        self.consecutive_failures.store(0, Ordering::SeqCst);
        let mut state = self.state.lock().await;
        *state = BreakerState::Closed;
    }

    /// Record a failed call: increment failures and trip if threshold met.
    /// Returns `true` if the breaker just tripped to Open.
    async fn record_failure(&self) -> bool {
        let prev = self.consecutive_failures.fetch_add(1, Ordering::SeqCst);
        let new_count = prev + 1;
        if new_count >= self.cfg.failure_threshold {
            let mut state = self.state.lock().await;
            *state = BreakerState::Open {
                opened_at: Instant::now(),
            };
            return true;
        }
        false
    }

    /// Check (and potentially transition) the breaker state before a call.
    /// Returns `Err(PatternError::CircuitOpen)` if the call should be
    /// rejected, or `Ok(())` if the call may proceed.
    async fn check_state(&self) -> Result<(), PatternError> {
        let mut state = self.state.lock().await;
        match &*state {
            BreakerState::Closed => Ok(()),
            BreakerState::Open { opened_at } => {
                if opened_at.elapsed() >= self.cfg.reset_after {
                    // Transition to HalfOpen — allow probe calls.
                    *state = BreakerState::HalfOpen {
                        probes_remaining: self.cfg.half_open_probes,
                    };
                    Ok(())
                } else {
                    Err(PatternError::CircuitOpen {
                        name: self.name.clone(),
                    })
                }
            }
            BreakerState::HalfOpen { probes_remaining } => {
                if *probes_remaining > 0 {
                    Ok(())
                } else {
                    Err(PatternError::CircuitOpen {
                        name: self.name.clone(),
                    })
                }
            }
        }
    }

    /// Consume one probe slot if we are in HalfOpen state.
    async fn consume_probe(&self) {
        let mut state = self.state.lock().await;
        if let BreakerState::HalfOpen { probes_remaining } = &mut *state {
            *probes_remaining = probes_remaining.saturating_sub(1);
        }
    }
}

// ---------------------------------------------------------------------------
// Pattern impl
// ---------------------------------------------------------------------------

#[async_trait]
impl Pattern for CircuitBreaker {
    fn kind(&self) -> PatternKind {
        self.inner.kind()
    }

    async fn execute(&self, input: Value, ctx: PatternCtx) -> Result<Value, PatternError> {
        // 1. Gate: reject immediately if Open (and not yet ready to probe).
        self.check_state().await?;

        // 2. If HalfOpen, consume a probe slot before the call.
        self.consume_probe().await;

        // 3. Execute inner pattern with timeout.
        let result = tokio::time::timeout(self.timeout, self.inner.execute(input, ctx)).await;

        match result {
            // Inner completed within timeout.
            Ok(Ok(value)) => {
                self.record_success().await;
                Ok(value)
            }
            // Inner returned an error within timeout.
            Ok(Err(err)) => {
                let tripped = self.record_failure().await;
                if tripped {
                    Err(PatternError::CircuitOpen {
                        name: self.name.clone(),
                    })
                } else {
                    Err(err)
                }
            }
            // Timeout elapsed.
            Err(_elapsed) => {
                self.record_failure().await;
                Err(PatternError::Timeout)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Ergonomic helper
// ---------------------------------------------------------------------------

/// Wrap a pattern with circuit-breaker + timeout behavior, returning an
/// `Arc<dyn Pattern>` suitable for registration.
pub fn wrap_with_breaker(
    name: impl Into<String>,
    pattern: Arc<dyn Pattern>,
    cfg: CircuitBreakerConfig,
    timeout: Duration,
) -> Arc<dyn Pattern> {
    CircuitBreaker::wrap(name, pattern, cfg, timeout)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::time::Duration;

    use async_trait::async_trait;
    use serde_json::json;

    use crate::patterns::{
        PatternCtx, PatternError, PatternKind, PatternRegistry, TaskServiceHandle,
    };

    // -- Configurable stub pattern ------------------------------------------

    /// A test pattern that can be configured to succeed, fail, or sleep.
    struct StubPattern {
        /// Number of times `execute` has been called.
        call_count: AtomicU32,
        /// If true, always return an error.
        should_fail: AtomicBool,
        /// If true, sleep for a very long time (for timeout tests).
        should_hang: AtomicBool,
    }

    impl StubPattern {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                call_count: AtomicU32::new(0),
                should_fail: AtomicBool::new(false),
                should_hang: AtomicBool::new(false),
            })
        }

        fn set_fail(&self, fail: bool) {
            self.should_fail.store(fail, Ordering::SeqCst);
        }

        fn set_hang(&self, hang: bool) {
            self.should_hang.store(hang, Ordering::SeqCst);
        }

        fn calls(&self) -> u32 {
            self.call_count.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl Pattern for StubPattern {
        fn kind(&self) -> PatternKind {
            PatternKind::Custom("stub".into())
        }

        async fn execute(&self, _input: Value, _ctx: PatternCtx) -> Result<Value, PatternError> {
            self.call_count.fetch_add(1, Ordering::SeqCst);

            if self.should_hang.load(Ordering::SeqCst) {
                // Sleep for an absurdly long time — the timeout should fire first.
                tokio::time::sleep(Duration::from_secs(3600 * 24)).await;
            }

            if self.should_fail.load(Ordering::SeqCst) {
                return Err(PatternError::Execution("stub failure".into()));
            }

            Ok(json!({"ok": true}))
        }
    }

    // -- Test helpers -------------------------------------------------------

    fn make_ctx() -> PatternCtx {
        struct DummyTaskService;

        #[async_trait]
        impl TaskServiceHandle for DummyTaskService {
            async fn submit(&self, _agent: &str, input: Value) -> Result<Value, PatternError> {
                Ok(input)
            }
        }

        PatternCtx {
            task_service: Arc::new(DummyTaskService),
            registry: Arc::new(PatternRegistry::new()),
            trace_id: "test-cb".into(),
            deadline: None,
        }
    }

    fn default_cfg() -> CircuitBreakerConfig {
        CircuitBreakerConfig {
            failure_threshold: 3,
            reset_after: Duration::from_secs(60),
            half_open_probes: 1,
        }
    }

    fn default_timeout() -> Duration {
        Duration::from_secs(30 * 60) // 30 minutes per REQ-ARCH-006
    }

    // -- Tests --------------------------------------------------------------

    /// TC-PAT-03: After 3 consecutive failures the breaker trips and the 4th
    /// call returns `CircuitOpen` WITHOUT calling the inner pattern a 4th time.
    #[tokio::test]
    async fn test_cb_trips_after_3_consecutive_failures_tc_pat_03() {
        let stub = StubPattern::new();
        stub.set_fail(true);

        let cb = CircuitBreaker::wrap("agent-x", stub.clone(), default_cfg(), default_timeout());

        // Calls 1-3: should each hit the inner and get errors back.
        // The third failure trips the breaker.
        for i in 0..3 {
            let result = cb.execute(json!(null), make_ctx()).await;
            assert!(result.is_err(), "call {} should fail", i + 1);
        }

        assert_eq!(
            stub.calls(),
            3,
            "inner must have been called exactly 3 times"
        );

        // Call 4: breaker is Open, inner must NOT be called.
        let result = cb.execute(json!(null), make_ctx()).await;
        assert!(
            matches!(&result, Err(PatternError::CircuitOpen { name }) if name == "agent-x"),
            "4th call should be CircuitOpen, got: {:?}",
            result,
        );
        assert_eq!(
            stub.calls(),
            3,
            "inner must still be 3 — the 4th call was short-circuited"
        );
    }

    /// TC-PAT-04: The 30-minute default timeout fires when the inner hangs.
    #[tokio::test]
    async fn test_cb_30_minute_default_timeout_tc_pat_04() {
        tokio::time::pause();

        let stub = StubPattern::new();
        stub.set_hang(true);

        let cb = CircuitBreaker::wrap("slow-agent", stub.clone(), default_cfg(), default_timeout());

        let handle = tokio::spawn({
            let cb = cb.clone();
            async move { cb.execute(json!(null), make_ctx()).await }
        });

        // Advance time past the 30-minute timeout.
        tokio::time::advance(Duration::from_secs(30 * 60 + 1)).await;

        let result = handle.await.unwrap();
        assert!(
            matches!(&result, Err(PatternError::Timeout)),
            "expected Timeout, got: {:?}",
            result,
        );
    }

    /// Two failures then a success should reset the consecutive counter and
    /// keep the breaker Closed.
    #[tokio::test]
    async fn test_cb_success_resets_consecutive_failures() {
        let stub = StubPattern::new();
        stub.set_fail(true);

        let cb = CircuitBreaker::wrap("resettable", stub.clone(), default_cfg(), default_timeout());

        // Two failures.
        let _ = cb.execute(json!(null), make_ctx()).await;
        let _ = cb.execute(json!(null), make_ctx()).await;

        // Now succeed.
        stub.set_fail(false);
        let result = cb.execute(json!(null), make_ctx()).await;
        assert!(result.is_ok(), "third call should succeed");

        // Counter should be 0 now.
        assert_eq!(
            cb.consecutive_failures.load(Ordering::SeqCst),
            0,
            "consecutive_failures must be 0 after success"
        );

        // Verify state is Closed.
        let state = cb.state.lock().await;
        assert!(
            matches!(&*state, BreakerState::Closed),
            "breaker should be Closed, got: {:?}",
            *state,
        );
    }

    /// Trip the breaker -> wait past reset_after -> probe succeeds -> Closed.
    #[tokio::test]
    async fn test_cb_half_open_probe_success_closes() {
        tokio::time::pause();

        let stub = StubPattern::new();
        stub.set_fail(true);

        let cb = CircuitBreaker::wrap(
            "half-open-ok",
            stub.clone(),
            default_cfg(),
            default_timeout(),
        );

        // Trip the breaker (3 failures).
        for _ in 0..3 {
            let _ = cb.execute(json!(null), make_ctx()).await;
        }

        // Advance past reset_after.
        tokio::time::advance(Duration::from_secs(61)).await;

        // Next call should be allowed (HalfOpen probe) and succeed.
        stub.set_fail(false);
        let result = cb.execute(json!(null), make_ctx()).await;
        assert!(result.is_ok(), "probe should succeed: {:?}", result);

        // Breaker should now be Closed.
        let state = cb.state.lock().await;
        assert!(
            matches!(&*state, BreakerState::Closed),
            "breaker should be Closed after successful probe, got: {:?}",
            *state,
        );
    }

    /// Trip the breaker -> wait past reset_after -> probe fails -> back to Open.
    #[tokio::test]
    async fn test_cb_half_open_probe_failure_reopens() {
        tokio::time::pause();

        let stub = StubPattern::new();
        stub.set_fail(true);

        let cb = CircuitBreaker::wrap(
            "half-open-fail",
            stub.clone(),
            default_cfg(),
            default_timeout(),
        );

        // Trip the breaker (3 failures).
        for _ in 0..3 {
            let _ = cb.execute(json!(null), make_ctx()).await;
        }

        // Advance past reset_after.
        tokio::time::advance(Duration::from_secs(61)).await;

        // Probe with failure still active.
        let result = cb.execute(json!(null), make_ctx()).await;
        assert!(result.is_err(), "probe should fail");

        // Breaker should be back to Open.
        let state = cb.state.lock().await;
        assert!(
            matches!(&*state, BreakerState::Open { .. }),
            "breaker should be Open after failed probe, got: {:?}",
            *state,
        );
    }

    /// Manual reset clears the breaker and allows calls through again.
    #[tokio::test]
    async fn test_cb_manual_reset_clears_state() {
        let stub = StubPattern::new();
        stub.set_fail(true);

        let cb = CircuitBreaker::wrap(
            "manual-reset",
            stub.clone(),
            default_cfg(),
            default_timeout(),
        );

        // Trip the breaker.
        for _ in 0..3 {
            let _ = cb.execute(json!(null), make_ctx()).await;
        }

        // Verify it's Open.
        let result = cb.execute(json!(null), make_ctx()).await;
        assert!(matches!(&result, Err(PatternError::CircuitOpen { .. })));

        // Reset manually.
        cb.reset().await;

        // Next call should go through (configure stub to succeed).
        stub.set_fail(false);
        let result = cb.execute(json!(null), make_ctx()).await;
        assert!(
            result.is_ok(),
            "call after reset should succeed: {:?}",
            result
        );
    }

    /// The `CircuitOpen` error display includes the agent name and the
    /// phrase "circuit breaker" (ERR-PAT-01 diagnostics requirement).
    #[tokio::test]
    async fn test_cb_error_message_matches_err_pat_01() {
        let err = PatternError::CircuitOpen {
            name: "agent-x".into(),
        };
        let msg = err.to_string();
        assert!(
            msg.contains("agent-x"),
            "error message must contain the agent name, got: {}",
            msg,
        );
        assert!(
            msg.contains("circuit breaker"),
            "error message must contain 'circuit breaker', got: {}",
            msg,
        );
    }
}
