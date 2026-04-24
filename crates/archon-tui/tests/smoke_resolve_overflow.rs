//! Smoke tests for resolve_overflow and default_overflow_action API.
//!
//! Tests the OverflowAction resolution logic for context overflow handling.

use archon_tui::screens::session_browser::{OverflowAction, SessionBrowser};

/// Test: resolve_overflow TruncateOldest(3) on 10 msgs -> 7 msgs
#[test]
fn smoke_resolve_overflow_truncate_oldest_3_of_10() {
    let messages: Vec<String> = (0..10).map(|i| format!("msg {}", i)).collect();
    let action = OverflowAction::TruncateOldest(3);
    let result = SessionBrowser::resolve_overflow(messages, &action);
    assert!(result.is_some(), "TruncateOldest should return Some");
    let remaining = result.unwrap();
    assert_eq!(remaining.len(), 7, "should have 7 messages remaining");
    assert_eq!(&remaining[0], "msg 3", "first remaining should be msg 3");
    assert_eq!(&remaining[6], "msg 9", "last remaining should be msg 9");
}

/// Test: resolve_overflow TruncateOldest(100) on 5 msgs -> empty vec
#[test]
fn smoke_resolve_overflow_truncate_oldest_100_of_5() {
    let messages: Vec<String> = (0..5).map(|i| format!("msg {}", i)).collect();
    let action = OverflowAction::TruncateOldest(100);
    let result = SessionBrowser::resolve_overflow(messages, &action);
    assert!(result.is_some(), "TruncateOldest should return Some");
    let remaining = result.unwrap();
    assert!(
        remaining.is_empty(),
        "should return empty vec when n >= messages.len()"
    );
}

/// Test: resolve_overflow SwitchModel -> None
#[test]
fn smoke_resolve_overflow_switch_model() {
    let messages: Vec<String> = (0..5).map(|i| format!("msg {}", i)).collect();
    let action = OverflowAction::SwitchModel("claude-3-5-haiku".to_string());
    let result = SessionBrowser::resolve_overflow(messages, &action);
    assert!(result.is_none(), "SwitchModel should return None");
}

/// Test: resolve_overflow Cancelled -> None
#[test]
fn smoke_resolve_overflow_cancelled() {
    let messages: Vec<String> = (0..5).map(|i| format!("msg {}", i)).collect();
    let action = OverflowAction::Cancelled;
    let result = SessionBrowser::resolve_overflow(messages, &action);
    assert!(result.is_none(), "Cancelled should return None");
}

/// Test: default_overflow_action(800, 1000) -> TruncateOldest(0)
#[test]
fn smoke_default_overflow_action_under_limit() {
    // 800 tokens with 1000 limit: 90% of limit = 900, so 800 <= 900 -> TruncateOldest(0)
    let action = SessionBrowser::default_overflow_action(800, 1000);
    match action {
        OverflowAction::TruncateOldest(n) => {
            assert_eq!(n, 0, "under limit should return TruncateOldest(0)");
        }
        other => panic!("expected TruncateOldest(0), got {:?}", other),
    }
}

/// Test: default_overflow_action(1100, 1000) -> TruncateOldest(n>0)
#[test]
fn smoke_default_overflow_action_over_limit() {
    // 1100 tokens with 1000 limit: 90% of limit = 900, so 1100 > 900 -> TruncateOldest(n>0)
    let action = SessionBrowser::default_overflow_action(1100, 1000);
    match action {
        OverflowAction::TruncateOldest(n) => {
            assert!(
                n > 0,
                "over limit should return TruncateOldest(n>0), got {}",
                n
            );
        }
        other => panic!("expected TruncateOldest(n>0), got {:?}", other),
    }
}
