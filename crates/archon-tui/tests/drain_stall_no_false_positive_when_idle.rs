use std::sync::atomic::Ordering;

use archon_tui::observability::{
    DEFAULT_DRAIN_STALL_THRESHOLD_MS, TUI_EVENT_LAST_DRAIN_UNIX_MS,
    reset_tui_drain_stall_state_for_tests, tui_event_pending_count, warn_if_drain_stalled,
};

#[test]
fn drain_stall_no_false_positive_when_idle() {
    reset_tui_drain_stall_state_for_tests();
    TUI_EVENT_LAST_DRAIN_UNIX_MS.store(
        now_unix_ms().saturating_sub(DEFAULT_DRAIN_STALL_THRESHOLD_MS + 1),
        Ordering::Relaxed,
    );

    assert_eq!(tui_event_pending_count(), 0);
    assert!(!warn_if_drain_stalled(DEFAULT_DRAIN_STALL_THRESHOLD_MS));
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
