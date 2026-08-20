//! Forking at an earlier message (#192).
//!
//! `fork_session` answers "carry on from here in a separate session".
//! `fork_session_at` answers "go back to before that and try something else",
//! which had no implementation, so the branch picker built for it had nothing
//! to call.

use archon_session::fork::{fork_session, fork_session_at};
use archon_session::storage::SessionStore;

fn store_with(messages: usize) -> (tempfile::TempDir, SessionStore, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = SessionStore::open(&dir.path().join("sessions.db")).expect("store");
    let session = store
        .create_session(&dir.path().to_string_lossy(), None, "claude-opus-5")
        .expect("session")
        .id;
    for index in 0..messages {
        store
            .save_message(
                &session,
                index as u64,
                &serde_json::json!({ "role": "user", "content": format!("m{index}") }).to_string(),
            )
            .expect("message");
    }
    (dir, store, session)
}

/// Inclusive: branching at message 1 keeps two messages, not one.
#[test]
fn a_branch_keeps_everything_through_the_chosen_message() {
    let (_dir, store, session) = store_with(5);

    let branch = fork_session_at(&store, &session, 1, None).expect("fork at");

    let kept = store.load_messages(&branch).expect("messages");
    assert_eq!(kept.len(), 2);
    assert!(kept[0].contains("m0"));
    assert!(kept[1].contains("m1"));
}

#[test]
fn branching_at_the_first_message_keeps_only_it() {
    let (_dir, store, session) = store_with(4);

    let branch = fork_session_at(&store, &session, 0, None).expect("fork at");

    assert_eq!(store.load_messages(&branch).expect("messages").len(), 1);
}

/// Asking to branch after the last message is asking for all of it, which is a
/// plain fork rather than an error.
#[test]
fn an_index_past_the_end_keeps_everything() {
    let (_dir, store, session) = store_with(3);

    let branch = fork_session_at(&store, &session, 99, None).expect("fork at");

    assert_eq!(store.load_messages(&branch).expect("messages").len(), 3);
}

/// The original is untouched: branching is not rewinding.
#[test]
fn the_source_session_keeps_all_of_its_messages() {
    let (_dir, store, session) = store_with(5);

    fork_session_at(&store, &session, 1, None).expect("fork at");

    assert_eq!(store.load_messages(&session).expect("messages").len(), 5);
}

#[test]
fn a_branch_records_its_parent_and_can_be_named() {
    let (_dir, store, session) = store_with(3);

    let branch = fork_session_at(&store, &session, 1, Some("other-approach")).expect("fork at");

    assert_eq!(store.get_parent(&branch).expect("parent"), Some(session));
    assert_eq!(
        store.get_name(&branch).expect("name").as_deref(),
        Some("other-approach")
    );
}

/// The whole-log path must be unchanged by the addition.
#[test]
fn a_plain_fork_still_copies_the_whole_log() {
    let (_dir, store, session) = store_with(4);

    let forked = fork_session(&store, &session, None).expect("fork");

    assert_eq!(store.load_messages(&forked).expect("messages").len(), 4);
}
