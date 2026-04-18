//! Integration tests for session_browser module.
//!
//! Tests SessionState and BranchPoint serialization and basic construction.

use archon_tui::screens::session_browser::{BranchPoint, SessionState};

/// Gate 1: Verify SessionState::new() produces default state.
#[test]
fn session_state_new_default() {
    let state = SessionState::new();
    assert!(state.current_id.is_none());
    assert!(state.branches.is_empty());
    assert_eq!(state.history_cursor, 0);
}

/// Gate 1: Verify SessionState::with_session(id) sets current_id.
#[test]
fn session_state_with_session_sets_current_id() {
    let state = SessionState::with_session("session-abc".to_string());
    assert_eq!(state.current_id, Some("session-abc".to_string()));
    assert!(state.branches.is_empty());
    assert_eq!(state.history_cursor, 0);
}

/// Gate 1: Verify SessionState roundtrips through serde_json.
#[test]
fn session_state_serde_roundtrip() {
    let state = SessionState::with_session("test-session".to_string());
    let json = serde_json::to_string(&state).expect("serialize SessionState");
    let decoded: SessionState = serde_json::from_str(&json).expect("deserialize SessionState");
    assert_eq!(decoded.current_id, Some("test-session".to_string()));
    assert!(decoded.branches.is_empty());
    assert_eq!(decoded.history_cursor, 0);
}

/// Gate 1: Verify BranchPoint roundtrips through serde_json with chrono DateTime.
#[test]
fn branch_point_serde_roundtrip() {
    use chrono::{DateTime, Utc};

    let point = BranchPoint {
        id: "bp-1".to_string(),
        parent_session: "parent-1".to_string(),
        branched_at_message: 42,
        label: "feature-x".to_string(),
        created_at: Utc::now(),
    };

    let json = serde_json::to_string(&point).expect("serialize BranchPoint");
    let decoded: BranchPoint = serde_json::from_str(&json).expect("deserialize BranchPoint");

    assert_eq!(decoded.id, "bp-1");
    assert_eq!(decoded.parent_session, "parent-1");
    assert_eq!(decoded.branched_at_message, 42);
    assert_eq!(decoded.label, "feature-x");
    // created_at should survive roundtrip (within 1 second tolerance)
    let diff = (decoded.created_at - point.created_at).num_seconds().abs();
    assert!(diff < 1, "created_at should be preserved within 1 second");
}

/// Gate 1: Verify Debug formatting on SessionState does not panic.
#[test]
fn session_state_debug_does_not_panic() {
    let state = SessionState::new();
    let debug_str = format!("{:?}", state);
    assert!(!debug_str.is_empty());
    // Should contain the struct name
    assert!(debug_str.contains("SessionState"));
}

/// Gate 1: Verify Debug formatting on BranchPoint does not panic.
#[test]
fn branch_point_debug_does_not_panic() {
    use chrono::Utc;

    let point = BranchPoint {
        id: "bp-debug".to_string(),
        parent_session: "parent-debug".to_string(),
        branched_at_message: 1,
        label: "test-label".to_string(),
        created_at: Utc::now(),
    };

    let debug_str = format!("{:?}", point);
    assert!(!debug_str.is_empty());
    assert!(debug_str.contains("BranchPoint"));
}

/// Gate 1: Verify SessionState with branches serializes correctly.
#[test]
fn session_state_with_branches_roundtrip() {
    use chrono::Utc;

    let state = SessionState::with_session("main-session".to_string());
    let json = serde_json::to_string(&state).expect("serialize");
    let decoded: SessionState = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(decoded.current_id, Some("main-session".to_string()));
}

/// Gate 1: Verify Default trait on SessionState.
#[test]
fn session_state_default_trait() {
    let default_state = SessionState::default();
    assert!(default_state.current_id.is_none());
    assert!(default_state.branches.is_empty());
    assert_eq!(default_state.history_cursor, 0);
}