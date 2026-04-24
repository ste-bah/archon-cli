//! Unit tests for ChannelMetrics
use archon_tui::observability::{ChannelMetrics, ChannelMetricsSnapshot};

fn make_fresh() -> ChannelMetrics {
    ChannelMetrics::new()
}

#[test]
fn new_returns_zeroed_backlog_depth() {
    let m = make_fresh();
    assert_eq!(
        m.backlog_depth.load(std::sync::atomic::Ordering::Relaxed),
        0
    );
}

#[test]
fn new_returns_zeroed_total_sent() {
    let m = make_fresh();
    assert_eq!(m.total_sent.load(std::sync::atomic::Ordering::Relaxed), 0);
}

#[test]
fn new_returns_zeroed_total_drained() {
    let m = make_fresh();
    assert_eq!(
        m.total_drained.load(std::sync::atomic::Ordering::Relaxed),
        0
    );
}

#[test]
fn new_returns_zeroed_max_batch_size() {
    let m = make_fresh();
    assert_eq!(
        m.max_batch_size.load(std::sync::atomic::Ordering::Relaxed),
        0
    );
}

#[test]
fn record_sent_increments_backlog_and_total_sent() {
    let m = make_fresh();
    m.record_sent();
    assert_eq!(
        m.backlog_depth.load(std::sync::atomic::Ordering::Relaxed),
        1
    );
    assert_eq!(m.total_sent.load(std::sync::atomic::Ordering::Relaxed), 1);
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
