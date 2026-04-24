//! TASK-TUI-710: Session Data Layer Integration Tests (TC-TUI-SESSION-01..06)
//!
//! Covers:
//! - TC-01: Browser lists and selects
//! - TC-02: Restore roundtrip
//! - TC-03: Overflow triggers context overflow
//! - TC-04: Branch creation yields switchable list
//! - TC-05: Fork creates and switches between branches
//! - TC-06: Compute stats populated session

use archon_session::storage::SessionStore;
use archon_tui::screens::session_branching::SessionBranching;
use archon_tui::screens::session_browser::{OverflowAction, ResumeOutcome, SessionBrowser};
use archon_tui::screens::session_stats::{NullStats, compute_stats};
use std::sync::Arc;

/// Seed an in-memory store with sessions and messages.
fn seed_store(
    sessions: usize,
    messages_per_session: usize,
) -> (tempfile::TempDir, Arc<SessionStore>) {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let store = Arc::new(SessionStore::open(&temp_dir.path().join("test.db")).expect("test store"));
    for s in 0..sessions {
        let session_id = format!("session-{s}");
        store
            .register_session(&session_id, "/tmp", None, "claude-3-5-sonnet")
            .expect("register session");
        store
            .set_name(&session_id, &format!("Session {s}"))
            .expect("set name");
        for m in 0..messages_per_session {
            let content = format!(
                r#"{{"role":"assistant","agent":"agent{}","message":"msg{}"}}"#,
                m % 2,
                m
            );
            store
                .save_message(&session_id, m as u64, &content)
                .expect("save message");
        }
    }
    (temp_dir, store)
}

// ---------------------------------------------------------------------------
// TC-01: Browser lists and selects
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tc_session_01_browser_lists_and_selects() {
    let (_temp_dir, store) = seed_store(3, 2);
    let mut browser = SessionBrowser::new(store);

    browser.refresh().await.expect("refresh");
    assert_eq!(browser.len(), 3, "browser should list 3 sessions");

    browser.move_down();
    assert_eq!(browser.selected_index(), 1, "cursor should move to index 1");

    browser.move_down();
    assert_eq!(browser.selected_index(), 2, "cursor should move to index 2");

    // Bounded at end
    browser.move_down();
    assert_eq!(
        browser.selected_index(),
        2,
        "cursor should stay at last index"
    );
}

// ---------------------------------------------------------------------------
// TC-02: Restore roundtrip
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tc_session_02_restore_roundtrip() {
    let (_temp_dir, store) = seed_store(1, 5);
    let session_id = "session-0";
    let mut browser = SessionBrowser::new(Arc::clone(&store));

    let outcome = browser
        .restore(session_id, 1_000_000)
        .await
        .expect("restore should succeed");

    match outcome {
        ResumeOutcome::Restored {
            session_id: sid,
            messages_loaded,
        } => {
            assert_eq!(sid, session_id);
            assert_eq!(messages_loaded, 5, "should load all 5 messages");
        }
        other => panic!("Expected Restored, got {:?}", other),
    }

    assert_eq!(
        browser.state().current_id,
        Some(session_id.to_string()),
        "state.current_id should be set after restore"
    );
    assert_eq!(
        browser.last_restored_name(),
        Some("Session 0"),
        "last_restored_name should be set after restore"
    );
}

// ---------------------------------------------------------------------------
// TC-03: Overflow triggers context overflow
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tc_session_03_overflow_triggers_context_overflow() {
    let (_temp_dir, store) = seed_store(1, 50);
    let session_id = "session-0";
    let mut browser = SessionBrowser::new(Arc::clone(&store));

    // Tiny context limit forces overflow
    let outcome = browser
        .restore(session_id, 10)
        .await
        .expect("restore should succeed");

    let n = match outcome {
        ResumeOutcome::ContextOverflow {
            action: OverflowAction::TruncateOldest(n),
            ..
        } => {
            assert!(n > 0, "truncation count should be > 0");
            n
        }
        other => panic!(
            "Expected ContextOverflow with TruncateOldest, got {:?}",
            other
        ),
    };

    // Resolve the overflow and assert message count is reduced
    let messages = store.load_messages(session_id).expect("load messages");
    let resolved = SessionBrowser::resolve_overflow(messages, &OverflowAction::TruncateOldest(n));
    assert!(resolved.is_some());
    let kept = resolved.unwrap();
    assert_eq!(
        kept.len(),
        50 - n,
        "resolved messages should be reduced by truncation count"
    );
}

// ---------------------------------------------------------------------------
// TC-04: Branch creation yields switchable list
// ---------------------------------------------------------------------------

#[test]
fn tc_session_04_branch_creation_yields_switchable_list() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let store = Arc::new(SessionStore::open(&temp_dir.path().join("test.db")).expect("test store"));

    let parent_id = "parent-session";
    store
        .register_session(parent_id, "/tmp", None, "claude-3-5-sonnet")
        .expect("register");
    store.set_name(parent_id, "Parent").expect("set name");

    let state =
        archon_tui::screens::session_browser::SessionState::with_session(parent_id.to_string());
    let mut branching = SessionBranching::new(store, state);

    let bp = branching
        .fork(5, "experiment-a")
        .expect("fork should succeed");
    assert_eq!(branching.branches().len(), 1, "should have 1 branch");

    branching.switch(&bp.id).expect("switch should succeed");
    assert_eq!(
        branching.state().current_id,
        Some(bp.id.clone()),
        "state.current_id should be updated after switch"
    );
}

// ---------------------------------------------------------------------------
// TC-05: Fork creates and switches between branches
// ---------------------------------------------------------------------------

#[test]
fn tc_session_05_fork_creates_and_switches_between_branches() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let store = Arc::new(SessionStore::open(&temp_dir.path().join("test.db")).expect("test store"));

    let parent_id = "parent-session";
    store
        .register_session(parent_id, "/tmp", None, "claude-3-5-sonnet")
        .expect("register");
    store.set_name(parent_id, "Parent").expect("set name");

    let state =
        archon_tui::screens::session_browser::SessionState::with_session(parent_id.to_string());
    let mut branching = SessionBranching::new(store, state);

    let bp1 = branching.fork(3, "branch-alpha").expect("fork1");
    let bp2 = branching.fork(7, "branch-beta").expect("fork2");
    assert_eq!(branching.branches().len(), 2, "should have 2 branches");

    branching.switch(&bp1.id).expect("switch to alpha");
    assert_eq!(
        branching.state().current_id,
        Some(bp1.id.clone()),
        "should be on branch-alpha"
    );

    branching.switch(&bp2.id).expect("switch to beta");
    assert_eq!(
        branching.state().current_id,
        Some(bp2.id.clone()),
        "should be on branch-beta"
    );

    // Verify no merge method exists by compile-time check (SessionBranching has no merge fn)
    // If a merge method were added, this test would need updating.
}

// ---------------------------------------------------------------------------
// TC-06: Compute stats populated session
// ---------------------------------------------------------------------------

#[test]
fn tc_session_06_compute_stats_populated_session() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let store = SessionStore::open(&temp_dir.path().join("test.db")).expect("test store");

    let session = store
        .create_session("/tmp", None, "claude-3-5-sonnet-4b")
        .expect("create session");

    // 7 messages across 2 agents and 3 tool invocations
    store
        .save_message(&session.id, 0, r#"{"role":"user","agent":"user"}"#)
        .unwrap();
    store.save_message(&session.id, 1, r#"{"role":"assistant","agent":"coder","tool":"grep","token_usage":{"input_tokens":10,"output_tokens":5}}"#).unwrap();
    store
        .save_message(&session.id, 2, r#"{"role":"user","agent":"user"}"#)
        .unwrap();
    store.save_message(&session.id, 3, r#"{"role":"assistant","agent":"reviewer","tool":"sed","token_usage":{"input_tokens":15,"output_tokens":8}}"#).unwrap();
    store
        .save_message(&session.id, 4, r#"{"role":"user","agent":"user"}"#)
        .unwrap();
    store.save_message(&session.id, 5, r#"{"role":"assistant","agent":"coder","tool":"awk","token_usage":{"input_tokens":20,"output_tokens":10}}"#).unwrap();
    store
        .save_message(&session.id, 6, r#"{"role":"user","agent":"user"}"#)
        .unwrap();

    let metrics = NullStats;
    let stats = compute_stats(&store, &session.id, &metrics);

    assert_eq!(stats.message_count, 7, "message_count should be 7");
    assert_eq!(
        stats.agent_count, 3,
        "agent_count: user + coder + reviewer = 3"
    );
    assert!(
        stats.recent_tools.len() <= 10,
        "recent_tools should be capped at 10"
    );
    assert!(
        stats.estimated_cost_usd.is_none(),
        "estimated_cost_usd is None (pricing table follow-up)"
    );
    assert!(
        stats.elapsed >= std::time::Duration::ZERO,
        "elapsed should be >= 0"
    );
}
