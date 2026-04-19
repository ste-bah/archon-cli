//! Smoke tests for SessionBranching::fork API.
//!
//! Verifies the fork branch operation:
//! - Success: fork with active session -> returns Ok(BranchPoint), visible_branches.len() increases
//! - Error: fork without active session -> returns Err
//!
//! These are permanent integration-test artifacts verifying the fork ABI contract.

use std::sync::Arc;
use archon_session::storage::SessionStore;
use archon_tui::screens::session_browser::SessionState;
use archon_tui::screens::session_branching::SessionBranching;
use tempfile::TempDir;

/// Smoke test 1: fork with active session succeeds and appends a branch point.
/// Verifies:
/// - fork() returns Ok(BranchPoint)
/// - returned BranchPoint has non-empty id and correct parent/label
/// - branches().len() increases by 1 after fork
#[test]
fn smoke_fork_with_active_session_returns_branchpoint_and_increments_count() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let store = Arc::new(
        SessionStore::open(&temp_dir.path().join("fork_test.db")).expect("open store"),
    );

    // Create a parent session so fork has a valid parent to fork from
    let parent_meta = store
        .create_session("/tmp", None, "gpt-4")
        .expect("create parent session");
    let parent_id = parent_meta.id.clone();

    // SessionState with current_id set (active session)
    let state = SessionState::with_session(parent_id.clone());
    let mut branching = SessionBranching::new(store, state);

    let initial_count = branching.branches().len();

    // Fork at message index 3 with label "smoke-branch"
    let result = branching.fork(3, "smoke-branch");
    assert!(result.is_ok(), "fork with active session should succeed, got: {:?}", result);

    let bp = result.unwrap();
    assert!(!bp.id.is_empty(), "BranchPoint.id should be non-empty");
    assert_eq!(bp.parent_session, parent_id, "BranchPoint.parent_session should match");
    assert_eq!(bp.branched_at_message, 3, "BranchPoint.branched_at_message should be 3");
    assert_eq!(bp.label, "smoke-branch", "BranchPoint.label should match");

    assert_eq!(
        branching.branches().len(),
        initial_count + 1,
        "visible_branches count should increase by 1 after fork"
    );
}

/// Smoke test 2: fork without active session returns an error.
/// Verifies:
/// - fork() returns Err (not Ok)
/// - error message contains "no active session"
#[test]
fn smoke_fork_without_active_session_returns_err() {
    let temp_dir = TempDir::new().expect("create temp dir");
    let store = Arc::new(
        SessionStore::open(&temp_dir.path().join("fork_no_session.db")).expect("open store"),
    );

    // SessionState with current_id = None (no active session)
    let state = SessionState::new();
    let mut branching = SessionBranching::new(store, state);

    let result = branching.fork(5, "orphan-branch");
    assert!(result.is_err(), "fork without active session should return Err, got: {:?}", result);

    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("no active session"),
        "error message should contain 'no active session', got: {}",
        err_msg
    );
}
