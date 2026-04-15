//! Channel observability instrumentation for AgentEvent channel.
//!
//! Provides backlog depth, throughput, and P95 send-to-render latency tracking.

use std::sync::atomic::{AtomicU64, Ordering};
use parking_lot::Mutex;
use hdrhistogram::Histogram;

/// Channel instrumentation metrics.
///
/// # Atomic ordering
/// All counters use `Relaxed` ordering — these are approximate figures
/// for observability only, not correctness-critical data.
#[derive(Debug)]
pub struct ChannelMetrics {
    /// Current in-flight messages (sent - drained).
    pub backlog_depth: AtomicU64,
    /// Total messages sent since startup.
    pub total_sent: AtomicU64,
    /// Total messages drained since startup.
    pub total_drained: AtomicU64,
    /// Largest single drain batch observed.
    pub max_batch_size: AtomicU64,
    /// P95 send-to-render latency in milliseconds.
    pub p95_send_to_render_ms: Mutex<Histogram<u64>>,
}

impl ChannelMetrics {
    /// Construct a zeroed metrics instance.
    pub fn new() -> Self {
        Self {
            backlog_depth: AtomicU64::new(0),
            total_sent: AtomicU64::new(0),
            total_drained: AtomicU64::new(0),
            max_batch_size: AtomicU64::new(0),
            // histogram min=1ms, max=60_000ms (1 min), 3 significant figures
            p95_send_to_render_ms: Mutex::new(
                // Histogram range: 1ms floor (sub-ms rounded up), 60_000ms ceiling
                // (1 min — any higher is a stuck-thread bug, not a latency sample).
                Histogram::new_with_bounds(1, 60_000, 3)
                    .expect("histogram bounds 1..60_000ms, 3 sigfig — spec-constant"),
            ),
        }
    }

    /// Record a single send event.
    #[inline]
    pub fn record_sent(&self) {
        self.total_sent.fetch_add(1, Ordering::Relaxed);
        self.backlog_depth.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a drain event with the given batch size.
    #[inline]
    pub fn record_drained(&self, batch_size: u64) {
        // Cap batch_size at current backlog to prevent underflow
        let current_backlog = self.backlog_depth.load(Ordering::Relaxed);
        let actual_drained = batch_size.min(current_backlog);
        if actual_drained > 0 {
            self.total_drained.fetch_add(actual_drained, Ordering::Relaxed);
            self.backlog_depth.fetch_sub(actual_drained, Ordering::Relaxed);
        }
        self.max_batch_size.fetch_max(batch_size, Ordering::Relaxed);
    }

    /// Record a send-to-render latency sample in milliseconds.
    #[inline]
    pub fn record_latency_ms(&self, ms: u64) {
        let mut guard = self.p95_send_to_render_ms.lock();
        // Silently ignore recording errors — histogram has bounded range
        let _ = guard.record(ms);
    }

    /// Take an atomic snapshot of all counters.
    #[inline]
    pub fn snapshot(&self) -> ChannelMetricsSnapshot {
        ChannelMetricsSnapshot {
            backlog_depth: self.backlog_depth.load(Ordering::Relaxed),
            total_sent: self.total_sent.load(Ordering::Relaxed),
            total_drained: self.total_drained.load(Ordering::Relaxed),
            max_batch_size: self.max_batch_size.load(Ordering::Relaxed),
            p95_send_to_render_ms: {
                let guard = self.p95_send_to_render_ms.lock();
                guard.value_at_percentile(95.0) as u64
            },
        }
    }
}

impl Default for ChannelMetrics {
    fn default() -> Self {
        Self::new()
    }
}

/// Immutable snapshot of ChannelMetrics counters.
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelMetricsSnapshot {
    pub backlog_depth: u64,
    pub total_sent: u64,
    pub total_drained: u64,
    pub max_batch_size: u64,
    pub p95_send_to_render_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_fresh() -> ChannelMetrics {
        ChannelMetrics::new()
    }

    #[test]
    fn new_returns_zeroed_backlog_depth() {
        let m = make_fresh();
        assert_eq!(m.backlog_depth.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn new_returns_zeroed_total_sent() {
        let m = make_fresh();
        assert_eq!(m.total_sent.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn new_returns_zeroed_total_drained() {
        let m = make_fresh();
        assert_eq!(m.total_drained.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn new_returns_zeroed_max_batch_size() {
        let m = make_fresh();
        assert_eq!(m.max_batch_size.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn record_sent_increments_backlog_and_total_sent() {
        let m = make_fresh();
        m.record_sent();
        assert_eq!(m.backlog_depth.load(Ordering::Relaxed), 1);
        assert_eq!(m.total_sent.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn snapshot_returns_current_values() {
        let m = make_fresh();
        m.record_sent();
        m.record_sent();
        m.record_drained(2);
        let snap = m.snapshot();
        assert_eq!(snap.backlog_depth, 0); // 2 sent - 2 drained
        assert_eq!(snap.total_sent, 2);
        assert_eq!(snap.total_drained, 2);
        assert_eq!(snap.max_batch_size, 2);
    }
}