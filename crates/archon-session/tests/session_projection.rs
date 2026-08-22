//! Acceptance coverage for session projections (#193 Phase B).

use std::sync::Arc;

use archon_session::projection::{SessionEvent, SessionProjection};
use archon_session::projection_stats::{SessionStatsProjection, SessionStatsState};
use archon_session::storage::SessionStore;

fn store() -> (tempfile::TempDir, SessionStore, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = SessionStore::open(&dir.path().join("sessions.db")).expect("store");
    let session = store
        .create_session(&dir.path().to_string_lossy(), None, "claude-opus-5")
        .expect("session")
        .id;
    (dir, store, session)
}

fn message(role: &str, text: &str) -> String {
    serde_json::json!({ "role": role, "content": text }).to_string()
}

/// The migration proof: a derived view that was recomputed by rescanning the
/// log is now served by a projection unit.
#[test]
fn session_stats_are_served_by_a_projection_unit() {
    let (_dir, store, session) = store();
    for (index, (role, text)) in [("user", "one"), ("assistant", "two"), ("user", "three")]
        .iter()
        .enumerate()
    {
        store
            .save_message(&session, index as u64, &message(role, text))
            .expect("message");
    }

    let projected = store
        .project(&session, &SessionStatsProjection)
        .expect("project");

    assert_eq!(projected.state.message_count, 3);
    assert_eq!(projected.state.user_messages, 2);
    assert_eq!(projected.state.assistant_messages, 1);
    assert_eq!(projected.through_seq, Some(2));
}

/// Resume must not refold the whole log. The second call sees a cache that is
/// already current, folds nothing, and says so.
#[test]
fn resume_rebuilds_from_the_cache_without_refolding() {
    let (_dir, store, session) = store();
    store
        .save_message(&session, 0, &message("user", "one"))
        .expect("message");

    let first = store
        .project(&session, &SessionStatsProjection)
        .expect("first");
    assert!(first.advanced, "the first fold must do work");

    let second = store
        .project(&session, &SessionStatsProjection)
        .expect("second");

    assert!(
        !second.advanced,
        "a session with no new events must not refold"
    );
    assert_eq!(second.state.message_count, 1, "and must still be correct");
    assert_eq!(second.through_seq, Some(0));
}

/// Only the new events are folded when the log grows.
#[test]
fn a_growing_log_folds_only_what_is_new() {
    let (_dir, store, session) = store();
    store
        .save_message(&session, 0, &message("user", "one"))
        .expect("message");
    store
        .project(&session, &SessionStatsProjection)
        .expect("first");

    store
        .save_message(&session, 1, &message("assistant", "two"))
        .expect("message");
    let second = store
        .project(&session, &SessionStatsProjection)
        .expect("second");

    assert!(second.advanced);
    assert_eq!(second.state.message_count, 2);
    assert_eq!(second.state.assistant_messages, 1);
    assert_eq!(second.through_seq, Some(1));
}

/// A unit that ignores every event does no downstream work: nothing is written
/// and the driver reports no advance, which is what makes folding every event
/// through every unit affordable.
#[test]
fn a_unit_that_ignores_every_event_does_no_work() {
    /// Interested in nothing at all.
    struct Indifferent;

    impl SessionProjection for Indifferent {
        type State = SessionStatsState;

        fn key(&self) -> &'static str {
            "indifferent.v1"
        }

        fn init(&self) -> Self::State {
            SessionStatsState::default()
        }

        fn apply(&self, state: Arc<Self::State>, _event: &SessionEvent) -> Arc<Self::State> {
            state
        }

        fn view(&self, _state: &Self::State) -> serde_json::Value {
            serde_json::Value::Null
        }
    }

    let (_dir, store, session) = store();
    for index in 0..5 {
        store
            .save_message(&session, index, &message("user", "x"))
            .expect("message");
    }

    let projected = store.project(&session, &Indifferent).expect("project");

    assert!(
        !projected.advanced,
        "a unit that returns the same state must not be treated as having changed it"
    );
    assert_eq!(projected.state.message_count, 0);
    assert_eq!(
        projected.through_seq,
        Some(4),
        "the fold still reaches the end of the log; it just changes nothing"
    );
}

/// A unit whose rules changed must be able to start again, because the cached
/// state was produced by the old ones.
#[test]
fn invalidating_a_projection_forces_a_refold() {
    let (_dir, store, session) = store();
    store
        .save_message(&session, 0, &message("user", "one"))
        .expect("message");
    store
        .project(&session, &SessionStatsProjection)
        .expect("first");

    store
        .invalidate_projection(&session, SessionStatsProjection.key())
        .expect("invalidate");

    let refolded = store
        .project(&session, &SessionStatsProjection)
        .expect("refold");
    assert!(refolded.advanced, "the cache should have been discarded");
    assert_eq!(refolded.state.message_count, 1);
}

#[test]
fn an_empty_session_projects_to_the_initial_state() {
    let (_dir, store, session) = store();

    let projected = store
        .project(&session, &SessionStatsProjection)
        .expect("project");

    assert_eq!(projected.state.message_count, 0);
    assert_eq!(projected.through_seq, None);
    assert!(!projected.advanced);
}

/// Two sessions fold independently, or one session's cache would answer for
/// another.
#[test]
fn projections_are_scoped_to_their_session() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = SessionStore::open(&dir.path().join("sessions.db")).expect("store");
    let one = store
        .create_session(&dir.path().to_string_lossy(), None, "claude-opus-5")
        .expect("one")
        .id;
    let two = store
        .create_session(&dir.path().to_string_lossy(), None, "claude-opus-5")
        .expect("two")
        .id;

    store
        .save_message(&one, 0, &message("user", "only in one"))
        .expect("message");
    store.project(&one, &SessionStatsProjection).expect("one");

    let projected = store.project(&two, &SessionStatsProjection).expect("two");

    assert_eq!(projected.state.message_count, 0);
}

// ---------------------------------------------------------------------------
// The cache must not outlive the log it was folded from (#200 Phase 4 follow-up)
// ---------------------------------------------------------------------------

/// `replace_messages` rewrites the log wholesale, and the driver resumes
/// *after* the cached seq — so without invalidation the cache keeps serving
/// state folded over messages that are no longer in the store. Compaction is
/// exactly when this happens, and it happens to every projection at once, not
/// just the one that thought to guard itself.
#[test]
fn a_replaced_log_does_not_serve_counts_from_the_log_it_replaced() {
    let (_dir, store, session) = store();
    for index in 0..6 {
        store
            .save_message(&session, index, &message("user", &format!("m{index}")))
            .expect("message");
    }
    let before = store
        .project(&session, &SessionStatsProjection)
        .expect("project");
    assert_eq!(before.state.message_count, 6);

    // What `/compact` does at the store: six messages become two.
    store
        .replace_messages(
            &session,
            &[
                message("user", "[Context Summary] the first five, distilled"),
                message("assistant", "acknowledged"),
            ],
        )
        .expect("replace");

    let after = store
        .project(&session, &SessionStatsProjection)
        .expect("project");
    assert_eq!(
        after.state.message_count, 2,
        "the cache served counts for messages the store no longer holds"
    );
    assert_eq!(after.state.user_messages, 1);
    assert_eq!(after.state.assistant_messages, 1);
}

/// `truncate_messages_after` is `/rewind`: the tail is gone and the counts
/// derived from it must go with it.
#[test]
fn a_truncated_log_does_not_serve_counts_for_the_tail_it_dropped() {
    let (_dir, store, session) = store();
    for index in 0..6 {
        store
            .save_message(&session, index, &message("user", &format!("m{index}")))
            .expect("message");
    }
    assert_eq!(
        store
            .project(&session, &SessionStatsProjection)
            .expect("project")
            .state
            .message_count,
        6
    );

    store
        .truncate_messages_after(&session, 2)
        .expect("truncate");

    assert_eq!(
        store
            .project(&session, &SessionStatsProjection)
            .expect("project")
            .state
            .message_count,
        3,
        "the cache served counts for messages the rewind removed"
    );
}

/// `/clear` empties the log. A projection that still reports the cleared
/// conversation's shape is reporting content the user asked to be rid of.
#[test]
fn a_cleared_log_does_not_serve_counts_from_before_the_clear() {
    let (_dir, store, session) = store();
    for index in 0..4 {
        store
            .save_message(&session, index, &message("user", &format!("m{index}")))
            .expect("message");
    }
    assert_eq!(
        store
            .project(&session, &SessionStatsProjection)
            .expect("project")
            .state
            .message_count,
        4
    );

    store.delete_all_messages(&session).expect("clear");

    let after = store
        .project(&session, &SessionStatsProjection)
        .expect("project");
    assert_eq!(
        after.state.message_count, 0,
        "the cache survived the clear and still describes the cleared conversation"
    );
}
