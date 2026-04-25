//! Tests for ResumeOutcome and OverflowAction enums.
//! EC-TUI-016 reference: ContextOverflow handling in session resume.

use archon_tui::screens::session_browser::{OverflowAction, ResumeOutcome};

#[test]
fn restored_outcome_construction_and_debug() {
    let outcome = ResumeOutcome::Restored {
        session_id: "test-session-123".into(),
        messages_loaded: 42,
    };
    let debug_str = format!("{:?}", outcome);
    assert!(
        debug_str.contains("Restored"),
        "Debug output should contain 'Restored': {}",
        debug_str
    );
}

#[test]
fn context_overflow_outcome_construction_and_debug() {
    let outcome = ResumeOutcome::ContextOverflow {
        estimated_tokens: 150_000,
        limit: 200_000,
        action: OverflowAction::TruncateOldest(50),
    };
    let debug_str = format!("{:?}", outcome);
    assert!(
        debug_str.contains("ContextOverflow"),
        "Debug output should contain 'ContextOverflow': {}",
        debug_str
    );
}

#[test]
fn not_found_outcome_construction_and_debug() {
    let outcome = ResumeOutcome::NotFound;
    let debug_str = format!("{:?}", outcome);
    assert!(
        debug_str.contains("NotFound"),
        "Debug output should contain 'NotFound': {}",
        debug_str
    );
}

#[test]
fn truncate_oldest_action_construction_and_debug() {
    let action = OverflowAction::TruncateOldest(100);
    let debug_str = format!("{:?}", action);
    assert!(
        debug_str.contains("TruncateOldest"),
        "Debug output should contain 'TruncateOldest': {}",
        debug_str
    );
}

#[test]
fn switch_model_action_construction_and_debug() {
    let action = OverflowAction::SwitchModel("gpt-5".into());
    let debug_str = format!("{:?}", action);
    assert!(
        debug_str.contains("SwitchModel"),
        "Debug output should contain 'SwitchModel': {}",
        debug_str
    );
}

#[test]
fn cancelled_action_construction_and_debug() {
    let action = OverflowAction::Cancelled;
    let debug_str = format!("{:?}", action);
    assert!(
        debug_str.contains("Cancelled"),
        "Debug output should contain 'Cancelled': {}",
        debug_str
    );
}

#[test]
fn restored_partial_eq_same_values() {
    let outcome1 = ResumeOutcome::Restored {
        session_id: "session-abc".into(),
        messages_loaded: 10,
    };
    let outcome2 = ResumeOutcome::Restored {
        session_id: "session-abc".into(),
        messages_loaded: 10,
    };
    assert_eq!(outcome1, outcome2);
}

#[test]
fn overflow_action_eq_holds() {
    // Test Eq holds for OverflowAction variants
    let action1 = OverflowAction::TruncateOldest(100);
    let action2 = OverflowAction::TruncateOldest(100);
    assert_eq!(action1, action2);

    let action3 = OverflowAction::SwitchModel("gpt-5".into());
    let action4 = OverflowAction::SwitchModel("gpt-5".into());
    assert_eq!(action3, action4);

    let action5 = OverflowAction::Cancelled;
    let action6 = OverflowAction::Cancelled;
    assert_eq!(action5, action6);

    // Different variants are not equal
    let action7 = OverflowAction::TruncateOldest(50);
    let action8 = OverflowAction::SwitchModel("gpt-4".into());
    assert_ne!(action7, action8);
}
