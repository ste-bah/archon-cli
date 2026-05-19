//! # `archon_tui::observability` — thin shim over `archon-observability`.
//!
//! TASK-AGS-OBS-901 lifted the owning definitions of [`ChannelMetrics`],
//! [`ChannelMetricsSnapshot`], [`format_prometheus`], [`serve_metrics_on`],
//! and [`serve_metrics`] out of this file into
//! [`archon_observability::metrics`]. OBS-905/906 had already lifted the
//! tracing-glue surface ([`init_tracing`], [`span_agent_turn`],
//! [`span_channel_send`], [`span_slash_dispatch`], [`RedactionLayer`]) to
//! `archon_observability::{tracing, redaction}`.
//!
//! Post-LIFT this file mostly re-exports the shared observability surface so
//! every existing `archon_tui::observability::…` call site (state.rs,
//! session.rs, unit + integration tests, benches) compiles unchanged. It also
//! retains the TUI-specific drain-side event counters below until those move
//! into `archon-observability`.
//!
//! See `crates/archon-observability/src/metrics.rs` for the metrics impl +
//! unit tests. `observability_tracing.rs` remains as an OBS-905-era shim
//! that re-exports the tracing surface; it stays in place until the same
//! wiring subtask retires it alongside this file.
//!
//! New cross-crate helpers should go into `archon-observability`; only
//! TUI-specific compatibility wiring belongs here.

pub use archon_observability::metrics::{
    ChannelMetrics, ChannelMetricsSnapshot, format_prometheus, serve_metrics, serve_metrics_on,
};
pub use archon_observability::task_registry::{
    TaskSnapshot, abort_alive_tasks, log_alive_tasks_after_cancel, register_abort_handle,
    reset_task_registry_for_tests, spawn_blocking_named, spawn_named, task_snapshots,
};

// OBS-905 / OBS-906 tracing surface. Re-exported from the in-tree
// observability_tracing shim so `archon_tui::observability::{init_tracing,
// span_*, RedactionLayer}` keeps resolving for every existing caller.
pub use crate::observability_tracing::{
    RedactionLayer, init_tracing, span_agent_turn, span_channel_send, span_slash_dispatch,
};

// ---------------------------------------------------------------------------
// TASK #218 TUI-EVENT-BACKPRESSURE-MONITORING
// ---------------------------------------------------------------------------
//
// Drain-side counters for the production `tui_event_rx` channel.
//
// `TuiEventSender` now bounds the queue and records dropped events at the
// producer side. The drain-side counter + stall detection below still captures
// the other practical concern: the render loop can stop returning to the event
// drain phase. Combined with the existing AgentEvent ChannelMetrics (already
// bilateral, exposed via /metrics), operators can compare rates: if AgentEvent
// drained_total grows but TUI drained_total stalls, rendering is the culprit.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

/// Total `TuiEvent`s drained from `tui_event_rx` since process start.
/// Read via Prometheus `/metrics` endpoint or directly for tests.
pub static TUI_EVENT_DRAINED_TOTAL: AtomicU64 = AtomicU64::new(0);

/// Approximate count of `TuiEvent`s queued in the render-loop input channel.
///
/// Maintained manually because the stall watchdog runs outside the receiver
/// owner. Senders increment only after an event is actually queued; the
/// receiver decrements when an event leaves the queue.
pub static TUI_EVENT_PENDING: AtomicUsize = AtomicUsize::new(0);

/// Unix milliseconds of the last `record_tui_event_drain()` call.
/// `0` means never drained. Used by `warn_if_drain_stalled` to detect
/// a stuck render loop (no events processed for >threshold_ms).
pub static TUI_EVENT_LAST_DRAIN_UNIX_MS: AtomicU64 = AtomicU64::new(0);

static TUI_EVENT_LAST_DRAIN_VARIANT: OnceLock<Mutex<&'static str>> = OnceLock::new();
static TUI_EVENT_DRAINED_BY_VARIANT: OnceLock<Mutex<BTreeMap<&'static str, u64>>> = OnceLock::new();
static TUI_EVENT_LAST_STALL_WARN_UNIX_MS: AtomicU64 = AtomicU64::new(0);
static TUI_EVENT_LAST_STALL_WARN_PENDING: AtomicUsize = AtomicUsize::new(0);
static LONG_RUNNING_WORKLOAD_COUNT: AtomicUsize = AtomicUsize::new(0);

pub const DEFAULT_DRAIN_STALL_THRESHOLD_MS: u64 = 10_000;
pub const LONG_RUNNING_DRAIN_STALL_THRESHOLD_MS: u64 = 90_000;
const DRAIN_STALL_WARN_REFRESH_MS: u64 = 60_000;

pub fn record_tui_event_enqueued() {
    TUI_EVENT_PENDING.fetch_add(1, Ordering::Relaxed);
}

pub fn record_tui_event_dequeued() {
    decrement_tui_event_pending();
}

pub fn record_tui_event_discarded() {
    decrement_tui_event_pending();
}

pub fn tui_event_pending_count() -> usize {
    TUI_EVENT_PENDING.load(Ordering::Relaxed)
}

pub fn mark_long_running_workload(reason: &str) {
    LONG_RUNNING_WORKLOAD_COUNT.fetch_add(1, Ordering::Relaxed);
    tracing::trace!(reason, "long-running TUI workload marked");
}

pub fn clear_long_running_workload() {
    let mut current = LONG_RUNNING_WORKLOAD_COUNT.load(Ordering::Relaxed);
    loop {
        if current == 0 {
            return;
        }
        match LONG_RUNNING_WORKLOAD_COUNT.compare_exchange_weak(
            current,
            current - 1,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => return,
            Err(actual) => current = actual,
        }
    }
}

pub fn current_drain_threshold_ms() -> u64 {
    if LONG_RUNNING_WORKLOAD_COUNT.load(Ordering::Relaxed) > 0 {
        LONG_RUNNING_DRAIN_STALL_THRESHOLD_MS
    } else {
        DEFAULT_DRAIN_STALL_THRESHOLD_MS
    }
}

pub struct LongRunningWorkloadGuard;

impl LongRunningWorkloadGuard {
    pub fn new(reason: &str) -> Self {
        mark_long_running_workload(reason);
        Self
    }
}

impl Drop for LongRunningWorkloadGuard {
    fn drop(&mut self) {
        clear_long_running_workload();
    }
}

/// Increment the drain counter and update last-drain timestamp.
///
/// Call this once per `event_rx.try_recv()` success in the render loop.
/// `Relaxed` ordering — observability data, not correctness-critical.
#[inline]
pub fn record_tui_event_drain(variant: &'static str) {
    reset_drain_stall_hysteresis();
    TUI_EVENT_DRAINED_TOTAL.fetch_add(1, Ordering::Relaxed);
    let now_ms = now_unix_ms();
    TUI_EVENT_LAST_DRAIN_UNIX_MS.store(now_ms, Ordering::Relaxed);
    *last_variant_cell()
        .lock()
        .expect("last TuiEvent drain variant lock") = variant;
    let mut counts = variant_counts_cell()
        .lock()
        .expect("TuiEvent drain variant counts lock");
    *counts.entry(variant).or_default() += 1;
    tracing::trace!(variant, "TuiEvent drain");
}

pub fn last_tui_event_drain_variant() -> Option<&'static str> {
    let variant = *last_variant_cell()
        .lock()
        .expect("last TuiEvent drain variant lock");
    if variant.is_empty() {
        None
    } else {
        Some(variant)
    }
}

pub fn tui_event_drain_count_for(variant: &'static str) -> u64 {
    variant_counts_cell()
        .lock()
        .expect("TuiEvent drain variant counts lock")
        .get(variant)
        .copied()
        .unwrap_or(0)
}

/// Emit `tracing::warn!` if no drain in `threshold_ms` AND at least one
/// drain has occurred. Returns `true` if a warn was emitted.
///
/// Pre-startup (`TUI_EVENT_LAST_DRAIN_UNIX_MS == 0`) returns `false` to
/// avoid spurious warns before the first event.
pub fn warn_if_drain_stalled(threshold_ms: u64) -> bool {
    let last = TUI_EVENT_LAST_DRAIN_UNIX_MS.load(Ordering::Relaxed);
    if last == 0 {
        return false;
    }
    let pending = tui_event_pending_count();
    if pending == 0 {
        reset_drain_stall_hysteresis();
        return false;
    }

    let now_ms = now_unix_ms();
    let stalled_ms = now_ms.saturating_sub(last);
    if stalled_ms < threshold_ms {
        return false;
    }

    let last_warned = TUI_EVENT_LAST_STALL_WARN_UNIX_MS.load(Ordering::Relaxed);
    let last_warned_pending = TUI_EVENT_LAST_STALL_WARN_PENDING.load(Ordering::Relaxed);
    let should_warn = last_warned == 0
        || pending > last_warned_pending
        || now_ms.saturating_sub(last_warned) >= DRAIN_STALL_WARN_REFRESH_MS;
    if !should_warn {
        return false;
    }

    TUI_EVENT_LAST_STALL_WARN_UNIX_MS.store(now_ms, Ordering::Relaxed);
    TUI_EVENT_LAST_STALL_WARN_PENDING.store(pending, Ordering::Relaxed);
    let total = TUI_EVENT_DRAINED_TOTAL.load(Ordering::Relaxed);
    let last_variant = last_tui_event_drain_variant().unwrap_or("unknown");
    ::tracing::warn!(
        stalled_ms,
        pending_events = pending,
        total_drained = total,
        last_variant,
        threshold_ms,
        "TuiEvent drain stalled — render loop may be stuck"
    );
    true
}

#[doc(hidden)]
pub fn reset_tui_drain_stall_state_for_tests() {
    TUI_EVENT_PENDING.store(0, Ordering::Relaxed);
    TUI_EVENT_LAST_DRAIN_UNIX_MS.store(0, Ordering::Relaxed);
    TUI_EVENT_LAST_STALL_WARN_UNIX_MS.store(0, Ordering::Relaxed);
    TUI_EVENT_LAST_STALL_WARN_PENDING.store(0, Ordering::Relaxed);
    LONG_RUNNING_WORKLOAD_COUNT.store(0, Ordering::Relaxed);
}

fn decrement_tui_event_pending() {
    let mut current = TUI_EVENT_PENDING.load(Ordering::Relaxed);
    loop {
        if current == 0 {
            return;
        }
        match TUI_EVENT_PENDING.compare_exchange_weak(
            current,
            current - 1,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => return,
            Err(actual) => current = actual,
        }
    }
}

fn reset_drain_stall_hysteresis() {
    TUI_EVENT_LAST_STALL_WARN_UNIX_MS.store(0, Ordering::Relaxed);
    TUI_EVENT_LAST_STALL_WARN_PENDING.store(0, Ordering::Relaxed);
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn last_variant_cell() -> &'static Mutex<&'static str> {
    TUI_EVENT_LAST_DRAIN_VARIANT.get_or_init(|| Mutex::new(""))
}

fn variant_counts_cell() -> &'static Mutex<BTreeMap<&'static str, u64>> {
    TUI_EVENT_DRAINED_BY_VARIANT.get_or_init(|| Mutex::new(BTreeMap::new()))
}

#[cfg(test)]
mod tui_drain_metric_tests {
    use super::*;
    use serial_test::serial;

    fn reset_all() {
        reset_tui_drain_stall_state_for_tests();
        TUI_EVENT_DRAINED_TOTAL.store(0, Ordering::Relaxed);
        *last_variant_cell()
            .lock()
            .expect("last TuiEvent drain variant lock") = "";
        variant_counts_cell()
            .lock()
            .expect("TuiEvent drain variant counts lock")
            .clear();
    }

    fn set_last_drain_overdue(threshold_ms: u64) {
        TUI_EVENT_LAST_DRAIN_UNIX_MS.store(
            now_unix_ms().saturating_sub(threshold_ms + 1),
            Ordering::Relaxed,
        );
    }

    // These tests mutate process-global drain metrics. Serialize them under
    // one lock so timestamp, pending, hysteresis, and variant state stay local.
    #[test]
    #[serial(tui_drain_metrics)]
    fn drain_counter_increments() {
        reset_all();
        let baseline = TUI_EVENT_DRAINED_TOTAL.load(Ordering::Relaxed);
        record_tui_event_drain("TextDelta");
        record_tui_event_drain("TextDelta");
        record_tui_event_drain("Done");
        let after = TUI_EVENT_DRAINED_TOTAL.load(Ordering::Relaxed);
        assert!(
            after >= baseline + 3,
            "drain counter must have grown by at least 3 (baseline={baseline}, after={after})"
        );
        assert_eq!(last_tui_event_drain_variant(), Some("Done"));
        assert!(tui_event_drain_count_for("TextDelta") >= 2);
    }

    #[test]
    #[serial(tui_drain_metrics)]
    fn drain_updates_timestamp() {
        reset_all();
        record_tui_event_drain("AgentActivity");
        let stamped = TUI_EVENT_LAST_DRAIN_UNIX_MS.load(Ordering::Relaxed);
        assert!(
            stamped > 0,
            "last-drain timestamp must be non-zero after drain"
        );
        assert_eq!(last_tui_event_drain_variant(), Some("AgentActivity"));
    }

    #[test]
    #[serial(tui_drain_metrics)]
    fn stall_warn_returns_false_before_first_drain() {
        reset_all();
        // Use a thread-isolated check by reading the static directly.
        // This test only validates the early-return branch when last==0;
        // we can't actually reset the global between tests, so just check
        // the threshold-not-exceeded branch by passing an enormous threshold.
        record_tui_event_drain("SessionRenamed");
        let huge = u64::MAX / 2;
        assert!(
            !warn_if_drain_stalled(huge),
            "huge threshold should never fire warn"
        );
    }

    #[test]
    #[serial(tui_drain_metrics)]
    fn warn_if_drain_stalled_returns_false_when_pending_is_zero() {
        reset_all();
        set_last_drain_overdue(DEFAULT_DRAIN_STALL_THRESHOLD_MS);

        assert!(!warn_if_drain_stalled(DEFAULT_DRAIN_STALL_THRESHOLD_MS));
    }

    #[test]
    #[serial(tui_drain_metrics)]
    fn warn_if_drain_stalled_returns_true_when_pending_and_overdue() {
        reset_all();
        record_tui_event_enqueued();
        set_last_drain_overdue(DEFAULT_DRAIN_STALL_THRESHOLD_MS);

        assert!(warn_if_drain_stalled(DEFAULT_DRAIN_STALL_THRESHOLD_MS));
    }

    #[test]
    #[serial(tui_drain_metrics)]
    fn warn_if_drain_stalled_does_not_fire_for_pre_startup() {
        reset_all();
        record_tui_event_enqueued();

        assert!(!warn_if_drain_stalled(DEFAULT_DRAIN_STALL_THRESHOLD_MS));
    }

    #[test]
    #[serial(tui_drain_metrics)]
    fn pending_count_increments_on_enqueued_decrements_on_drain() {
        reset_all();
        let (tx, mut rx) = crate::event_channel::bounded_tui_event_channel_with_capacity(4);

        tx.send(crate::events::TuiEvent::TextDelta("one".into()))
            .expect("queue first event");
        tx.send(crate::events::TuiEvent::TextDelta("two".into()))
            .expect("queue second event");
        tx.send(crate::events::TuiEvent::TextDelta("three".into()))
            .expect("queue third event");

        rx.try_recv().expect("drain first event");
        rx.try_recv().expect("drain second event");

        assert_eq!(tui_event_pending_count(), 1);
    }

    #[test]
    #[serial(tui_drain_metrics)]
    fn pending_count_saturates_at_zero() {
        reset_all();

        record_tui_event_discarded();

        assert_eq!(tui_event_pending_count(), 0);
    }

    #[test]
    #[serial(tui_drain_metrics)]
    fn current_drain_threshold_ms_changes_with_workload_marker() {
        reset_all();
        let default = DEFAULT_DRAIN_STALL_THRESHOLD_MS;
        let long = LONG_RUNNING_DRAIN_STALL_THRESHOLD_MS;

        assert_eq!(current_drain_threshold_ms(), default);
        mark_long_running_workload("test");
        assert_eq!(current_drain_threshold_ms(), long);
        clear_long_running_workload();
        assert_eq!(current_drain_threshold_ms(), default);
    }

    #[test]
    #[serial(tui_drain_metrics)]
    fn warn_hysteresis_suppresses_duplicate_warnings_within_60s() {
        reset_all();
        record_tui_event_enqueued();
        set_last_drain_overdue(DEFAULT_DRAIN_STALL_THRESHOLD_MS);

        assert!(warn_if_drain_stalled(DEFAULT_DRAIN_STALL_THRESHOLD_MS));
        assert!(!warn_if_drain_stalled(DEFAULT_DRAIN_STALL_THRESHOLD_MS));
    }

    #[test]
    #[serial(tui_drain_metrics)]
    fn warn_hysteresis_clears_after_pending_count_grows() {
        reset_all();
        record_tui_event_enqueued();
        set_last_drain_overdue(DEFAULT_DRAIN_STALL_THRESHOLD_MS);

        assert!(warn_if_drain_stalled(DEFAULT_DRAIN_STALL_THRESHOLD_MS));
        record_tui_event_enqueued();
        assert!(warn_if_drain_stalled(DEFAULT_DRAIN_STALL_THRESHOLD_MS));
    }

    #[test]
    #[serial(tui_drain_metrics)]
    fn warn_hysteresis_refreshes_after_60_seconds() {
        reset_all();
        record_tui_event_enqueued();
        set_last_drain_overdue(DEFAULT_DRAIN_STALL_THRESHOLD_MS);

        assert!(warn_if_drain_stalled(DEFAULT_DRAIN_STALL_THRESHOLD_MS));
        TUI_EVENT_LAST_STALL_WARN_UNIX_MS.store(
            now_unix_ms().saturating_sub(DRAIN_STALL_WARN_REFRESH_MS + 1),
            Ordering::Relaxed,
        );
        assert!(warn_if_drain_stalled(DEFAULT_DRAIN_STALL_THRESHOLD_MS));
    }

    #[test]
    #[serial(tui_drain_metrics)]
    fn long_running_workload_guard_clears_on_drop() {
        reset_all();
        let default = DEFAULT_DRAIN_STALL_THRESHOLD_MS;

        {
            let _guard = LongRunningWorkloadGuard::new("test");
            assert_eq!(
                current_drain_threshold_ms(),
                LONG_RUNNING_DRAIN_STALL_THRESHOLD_MS
            );
        }

        assert_eq!(current_drain_threshold_ms(), default);
    }

    #[test]
    #[serial(tui_drain_metrics)]
    fn long_running_workload_guard_clears_on_unwind() {
        reset_all();
        let default = DEFAULT_DRAIN_STALL_THRESHOLD_MS;

        let result = std::panic::catch_unwind(|| {
            let _guard = LongRunningWorkloadGuard::new("panic-test");
            panic!("intentional guard cleanup test");
        });

        assert!(result.is_err());
        assert_eq!(current_drain_threshold_ms(), default);
    }
}
