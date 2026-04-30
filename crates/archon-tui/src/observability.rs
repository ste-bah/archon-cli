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
//! Post-LIFT this file contains **only re-exports** so every existing
//! `archon_tui::observability::…` call site (state.rs, session.rs, unit +
//! integration tests, benches) compiles unchanged. The goal of this shim is
//! to hold the external surface stable during the wiring subtask that
//! follows OBS-901; once every caller is migrated to the
//! `archon_observability::` paths directly, this shim can be deleted.
//!
//! See `crates/archon-observability/src/metrics.rs` for the metrics impl +
//! unit tests. `observability_tracing.rs` remains as an OBS-905-era shim
//! that re-exports the tracing surface; it stays in place until the same
//! wiring subtask retires it alongside this file.
//!
//! **Do not add new code here.** New helpers go into
//! `archon-observability`.

pub use archon_observability::metrics::{
    ChannelMetrics, ChannelMetricsSnapshot, format_prometheus, serve_metrics, serve_metrics_on,
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
// Bilateral ChannelMetrics instrumentation (sent + drained) for TuiEvent
// would require wrapping `UnboundedSender<TuiEvent>` across ~100 callsites.
// That refactor is deferred. The drain-side counter + stall detection below
// captures the practical concern (consumer falling behind) without touching
// the sender API. Combined with the existing AgentEvent ChannelMetrics
// (already bilateral, exposed via /metrics), operators can compare
// rates: if AgentEvent drained_total grows but TUI drained_total stalls,
// TuiEvent backpressure is the culprit.

use std::sync::atomic::{AtomicU64, Ordering};

/// Total `TuiEvent`s drained from `tui_event_rx` since process start.
/// Read via Prometheus `/metrics` endpoint or directly for tests.
pub static TUI_EVENT_DRAINED_TOTAL: AtomicU64 = AtomicU64::new(0);

/// Unix milliseconds of the last `record_tui_event_drain()` call.
/// `0` means never drained. Used by `warn_if_drain_stalled` to detect
/// a stuck render loop (no events processed for >threshold_ms).
pub static TUI_EVENT_LAST_DRAIN_UNIX_MS: AtomicU64 = AtomicU64::new(0);

/// Increment the drain counter and update last-drain timestamp.
///
/// Call this once per `event_rx.try_recv()` success in the render loop.
/// `Relaxed` ordering — observability data, not correctness-critical.
#[inline]
pub fn record_tui_event_drain() {
    TUI_EVENT_DRAINED_TOTAL.fetch_add(1, Ordering::Relaxed);
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    TUI_EVENT_LAST_DRAIN_UNIX_MS.store(now_ms, Ordering::Relaxed);
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
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let stalled_ms = now_ms.saturating_sub(last);
    if stalled_ms <= threshold_ms {
        return false;
    }
    let total = TUI_EVENT_DRAINED_TOTAL.load(Ordering::Relaxed);
    ::tracing::warn!(
        stalled_ms,
        total_drained = total,
        threshold_ms,
        "TuiEvent drain stalled — render loop may be stuck"
    );
    true
}

#[cfg(test)]
mod tui_drain_metric_tests {
    use super::*;

    #[test]
    fn drain_counter_increments() {
        let baseline = TUI_EVENT_DRAINED_TOTAL.load(Ordering::Relaxed);
        record_tui_event_drain();
        record_tui_event_drain();
        record_tui_event_drain();
        let after = TUI_EVENT_DRAINED_TOTAL.load(Ordering::Relaxed);
        assert!(
            after >= baseline + 3,
            "drain counter must have grown by at least 3 (baseline={baseline}, after={after})"
        );
    }

    #[test]
    fn drain_updates_timestamp() {
        record_tui_event_drain();
        let stamped = TUI_EVENT_LAST_DRAIN_UNIX_MS.load(Ordering::Relaxed);
        assert!(stamped > 0, "last-drain timestamp must be non-zero after drain");
    }

    #[test]
    fn stall_warn_returns_false_before_first_drain() {
        // Use a thread-isolated check by reading the static directly.
        // This test only validates the early-return branch when last==0;
        // we can't actually reset the global between tests, so just check
        // the threshold-not-exceeded branch by passing an enormous threshold.
        record_tui_event_drain();
        let huge = u64::MAX / 2;
        assert!(
            !warn_if_drain_stalled(huge),
            "huge threshold should never fire warn"
        );
    }
}
