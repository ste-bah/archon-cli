use std::sync::atomic::Ordering;

use archon_tui::app::TuiEvent;
use archon_tui::event_channel::bounded_tui_event_channel_with_capacity;
use archon_tui::observability::{
    DEFAULT_DRAIN_STALL_THRESHOLD_MS, TUI_EVENT_LAST_DRAIN_UNIX_MS,
    reset_tui_drain_stall_state_for_tests, tui_event_pending_count, warn_if_drain_stalled,
};

#[test]
fn drain_stall_fires_when_events_pile_up() {
    reset_tui_drain_stall_state_for_tests();
    let (tx, _rx) = bounded_tui_event_channel_with_capacity(8);

    for idx in 0..5 {
        tx.send(TuiEvent::TextDelta(format!("event-{idx}")))
            .expect("queue event");
    }
    TUI_EVENT_LAST_DRAIN_UNIX_MS.store(
        now_unix_ms().saturating_sub(DEFAULT_DRAIN_STALL_THRESHOLD_MS + 1),
        Ordering::Relaxed,
    );

    assert_eq!(tui_event_pending_count(), 5);
    assert!(warn_if_drain_stalled(DEFAULT_DRAIN_STALL_THRESHOLD_MS));
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
