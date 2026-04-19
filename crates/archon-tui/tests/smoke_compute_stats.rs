//! Smoke tests for compute_stats API (TASK-TUI-709 GATE 5).
//!
//! Verifies:
//! - compute_stats with missing session returns empty stats (message_count=0)
//! - compute_stats with populated session returns correct counts
//! - NullStats returns all zeros

use archon_tui::screens::session_stats::{compute_stats, NullStats, StatsSource};

/// Smoke test: compute_stats with missing session returns empty stats.
#[test]
fn smoke_compute_stats_missing_session_returns_empty() {
    let temp_dir = std::env::temp_dir();
    let db_path = temp_dir.join("smoke_compute_stats_missing.db");
    let store = archon_session::storage::SessionStore::open(&db_path).unwrap();

    let stats = compute_stats(&store, "nonexistent-session-id", &NullStats);

    assert_eq!(stats.message_count, 0, "missing session → message_count must be 0");
    assert_eq!(stats.agent_count, 0, "missing session → agent_count must be 0");
    assert!(stats.recent_commands.is_empty());
    assert!(stats.recent_agents.is_empty());
    assert!(stats.recent_tools.is_empty());

    std::fs::remove_file(&db_path).ok();
}

/// Smoke test: compute_stats with populated session returns correct counts.
#[test]
fn smoke_compute_stats_populated_session_returns_correct_counts() {
    let temp_dir = std::env::temp_dir();
    let db_path = temp_dir.join("smoke_compute_stats_populated.db");
    let store = archon_session::storage::SessionStore::open(&db_path).unwrap();

    let session = store.create_session("/tmp", None, "claude-3-5-sonnet-4b").unwrap();

    store.save_message(&session.id, 0, r#"{"role":"user","agent":"user"}"#).unwrap();
    store.save_message(&session.id, 1, r#"{"role":"assistant","agent":"coder"}"#).unwrap();
    store.save_message(&session.id, 2, r#"{"role":"user","agent":"user"}"#).unwrap();
    store.save_message(&session.id, 3, r#"{"role":"assistant","agent":"reviewer"}"#).unwrap();

    let stats = compute_stats(&store, &session.id, &NullStats);

    assert_eq!(stats.message_count, 4, "populated session → message_count must be 4");
    assert_eq!(stats.agent_count, 3, "3 distinct agents → agent_count must be 3");

    std::fs::remove_file(&db_path).ok();
}

/// Smoke test: NullStats returns all zeros.
#[test]
fn smoke_null_stats_returns_all_zeros() {
    let ns = NullStats;
    assert_eq!(ns.total_sent(), 0, "total_sent must be 0");
    assert_eq!(ns.total_drained(), 0, "total_drained must be 0");
    assert_eq!(ns.backlog_depth(), 0, "backlog_depth must be 0");
}