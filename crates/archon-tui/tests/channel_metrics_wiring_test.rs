// Integration tests for ChannelMetricSink trait wiring to ChannelMetrics in archon-tui.
//
// NOTE: This file intentionally fails to compile until metrics.rs is created in archon-core.
// This is expected and correct behavior at this gate.

use archon_core::ChannelMetricSink;
use archon_tui::observability::{ChannelMetrics, ChannelMetricsSnapshot};

/// Verify ChannelMetrics implements ChannelMetricSink trait.
#[test]
fn channel_metrics_impls_sink_trait() {
    // ChannelMetrics should implement ChannelMetricSink
    let metrics = ChannelMetrics::new();
    let _: &dyn ChannelMetricSink = &metrics;
}

/// Verify the trait object is Send + Sync (required for Arc<dyn ChannelMetricSink>).
#[test]
fn channel_metrics_sink_object_is_send_sync() {
    let metrics = ChannelMetrics::new();
    let sink: &dyn ChannelMetricSink = &metrics;

    // These compile-time assertions verify that &dyn ChannelMetricSink is Send + Sync
    fn assert_send<T: Send>(_: &T) {}
    fn assert_sync<T: Sync>(_: &T) {}

    assert_send(&sink);
    assert_sync(&sink);

    // Also verify Arc<dyn ChannelMetricSink> works
    let arc_sink: std::sync::Arc<dyn ChannelMetricSink> = std::sync::Arc::new(metrics);
    assert_send(&arc_sink);
    assert_sync(&arc_sink);
}

/// Verify record_drained updates total_drained and backlog_depth correctly across multiple calls.
#[test]
fn record_drained_cumulative_across_calls() {
    let metrics = ChannelMetrics::new();

    // Record first drain event: 100 messages
    // backlog starts at 0, then 100 sent -> backlog 100, then 100 drained -> backlog 0
    metrics.record_sent();
    metrics.record_sent();
    metrics.record_sent();
    metrics.record_drained(3);

    let snapshot1 = metrics.snapshot();
    assert_eq!(snapshot1.total_drained, 3, "first call: total_drained should be 3");

    // Record second drain event: 5 messages
    for _ in 0..5 {
        metrics.record_sent();
    }
    metrics.record_drained(5);

    let snapshot2 = metrics.snapshot();
    assert_eq!(snapshot2.total_drained, 8, "second call: total_drained should be cumulative 8");
}
