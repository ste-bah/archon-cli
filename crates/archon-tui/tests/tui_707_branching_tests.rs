//! Integration tests for session_branching module (TASK-TUI-707).
//!
//! Tests SessionBranching struct with SessionStore and SessionState.
//!
//! Gate 1: tests-written-first — test file exists BEFORE implementation.

use archon_session::storage::SessionStore;
use archon_tui::screens::session_branching::SessionBranching;
use archon_tui::screens::session_browser::{BranchPoint, SessionState};
use chrono::{DateTime, Utc};
use std::sync::Arc;
use tempfile::TempDir;

/// Gate 1: Verify SessionBranching::new(store, state) constructs correctly.
#[test]
fn test_new_with_store_and_state() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let store_path = temp_dir.path().join("test.db");
    let store = SessionStore::open(&store_path).expect("open store");

    let state = SessionState::with_session("test-session".to_string());
    let branching = SessionBranching::new(Arc::new(store), state.clone());

    assert_eq!(
        branching.state().current_id,
        Some("test-session".to_string())
    );
}

/// Gate 1: Verify switch("known_id") returns Ok and updates state.current_id.
#[test]
fn test_switch_to_known_branch_updates_state() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let store_path = temp_dir.path().join("test.db");
    let store = SessionStore::open(&store_path).expect("open store");

    let branch = BranchPoint {
        id: "known_id".to_string(),
        parent_session: "parent-1".to_string(),
        branched_at_message: 5,
        label: "feature-branch".to_string(),
        created_at: Utc::now(),
    };

    let mut state = SessionState::new();
    state.branches.push(branch);

    let mut branching = SessionBranching::new(Arc::new(store), state);
    let result = branching.switch("known_id");

    assert!(result.is_ok(), "switch to known branch should succeed");
    assert_eq!(
        branching.state().current_id,
        Some("known_id".to_string()),
        "state.current_id should be updated to known_id"
    );
}

/// Gate 1: Verify switch("unknown_id") returns Err with "unknown branch" message.
#[test]
fn test_switch_to_unknown_branch_errs() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let store_path = temp_dir.path().join("test.db");
    let store = SessionStore::open(&store_path).expect("open store");

    let branch = BranchPoint {
        id: "known_id".to_string(),
        parent_session: "parent-1".to_string(),
        branched_at_message: 5,
        label: "feature-branch".to_string(),
        created_at: Utc::now(),
    };

    let mut state = SessionState::new();
    state.branches.push(branch);

    let mut branching = SessionBranching::new(Arc::new(store), state);
    let result = branching.switch("unknown_id");

    assert!(result.is_err(), "switch to unknown branch should fail");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("unknown branch"),
        "error message should contain 'unknown branch', got: {}",
        err_msg
    );
}

/// Gate 1: Verify branches() returns slice of visible_branches.
#[test]
fn test_branches_returns_visible_branches() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let store_path = temp_dir.path().join("test.db");
    let store = SessionStore::open(&store_path).expect("open store");

    let branch1 = BranchPoint {
        id: "branch-1".to_string(),
        parent_session: "parent-1".to_string(),
        branched_at_message: 5,
        label: "feature-one".to_string(),
        created_at: Utc::now(),
    };

    let branch2 = BranchPoint {
        id: "branch-2".to_string(),
        parent_session: "parent-1".to_string(),
        branched_at_message: 10,
        label: "feature-two".to_string(),
        created_at: Utc::now(),
    };

    let mut state = SessionState::new();
    state.branches.push(branch1.clone());
    state.branches.push(branch2.clone());

    let branching = SessionBranching::new(Arc::new(store), state);
    let visible = branching.branches();

    assert_eq!(visible.len(), 2, "branches() should return 2 branches");
    assert_eq!(visible[0].id, "branch-1");
    assert_eq!(visible[1].id, "branch-2");
}
