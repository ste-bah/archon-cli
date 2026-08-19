//! Acceptance coverage for per-message feedback (#193 Phase C).

use archon_session::feedback::{FeedbackError, Rating};
use archon_session::storage::{SessionError, SessionStore};

fn store() -> (tempfile::TempDir, SessionStore, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = SessionStore::open(&dir.path().join("sessions.db")).expect("store");
    let session = store
        .create_session(&dir.path().to_string_lossy(), None, "claude-opus-5")
        .expect("session")
        .id;
    (dir, store, session)
}

#[test]
fn a_rating_can_be_set_changed_and_cleared() {
    let (_dir, store, session) = store();

    let set = store
        .set_feedback(&session, "msg-1", Rating::Positive, Some("useful"), None)
        .expect("first rating");
    assert_eq!(set.rating, Rating::Positive);
    assert_eq!(set.note.as_deref(), Some("useful"));

    let changed = store
        .set_feedback(
            &session,
            "msg-1",
            Rating::Negative,
            None,
            Some(&set.version),
        )
        .expect("change");
    assert_eq!(changed.rating, Rating::Negative);
    assert_eq!(changed.note, None, "the note was withdrawn with the rating");
    assert_ne!(
        changed.version, set.version,
        "a material update must replace the token, or the next writer's \
         compare-and-set is meaningless"
    );
    assert_eq!(
        changed.created_at, set.created_at,
        "changing a rating is not creating one"
    );

    store
        .clear_feedback(&session, "msg-1", Some(&changed.version))
        .expect("clear");
    assert_eq!(store.feedback(&session, "msg-1").expect("read"), None);
}

/// The TUI and the web workbench can both be open on one session. Without the
/// token, whichever wrote last would silently discard the other.
#[test]
fn a_concurrent_edit_is_refused_rather_than_overwritten() {
    let (_dir, store, session) = store();

    let first = store
        .set_feedback(&session, "msg-1", Rating::Positive, None, None)
        .expect("first");

    // A second window wrote while the first was still holding its version.
    store
        .set_feedback(
            &session,
            "msg-1",
            Rating::Negative,
            Some("changed my mind"),
            Some(&first.version),
        )
        .expect("second window");

    let error = store
        .set_feedback(
            &session,
            "msg-1",
            Rating::Positive,
            None,
            Some(&first.version),
        )
        .expect_err("the stale token must be refused");

    match error {
        SessionError::Feedback(FeedbackError::Conflict(current)) => {
            assert_eq!(
                current.rating,
                Rating::Negative,
                "the refusal must carry what is actually there, so a caller can \
                 show both rather than just reporting a collision"
            );
            assert_eq!(current.note.as_deref(), Some("changed my mind"));
        }
        other => panic!("expected a conflict, got {other:?}"),
    }
}

/// Rating something that already has a rating, without having read it, is the
/// same mistake as a stale token.
#[test]
fn writing_without_a_token_over_an_existing_rating_is_refused() {
    let (_dir, store, session) = store();
    store
        .set_feedback(&session, "msg-1", Rating::Positive, None, None)
        .expect("first");

    let error = store
        .set_feedback(&session, "msg-1", Rating::Negative, None, None)
        .expect_err("must refuse");

    assert!(matches!(
        error,
        SessionError::Feedback(FeedbackError::Conflict(_))
    ));
}

#[test]
fn updating_a_rating_that_no_longer_exists_says_so() {
    let (_dir, store, session) = store();
    let set = store
        .set_feedback(&session, "msg-1", Rating::Positive, None, None)
        .expect("first");
    store
        .clear_feedback(&session, "msg-1", Some(&set.version))
        .expect("clear");

    let error = store
        .set_feedback(
            &session,
            "msg-1",
            Rating::Negative,
            None,
            Some(&set.version),
        )
        .expect_err("must refuse");

    assert!(matches!(
        error,
        SessionError::Feedback(FeedbackError::NotFound)
    ));
}

/// Withdrawing nothing is what the caller wanted, not an error to handle.
#[test]
fn clearing_an_unrated_message_is_harmless() {
    let (_dir, store, session) = store();
    store
        .clear_feedback(&session, "never-rated", None)
        .expect("no-op");
}

/// The learning layer consumes the whole set, and two readers must agree on
/// the order.
#[test]
fn every_rating_in_a_session_can_be_read_back_in_a_stable_order() {
    let (_dir, store, session) = store();
    for id in ["msg-3", "msg-1", "msg-2"] {
        store
            .set_feedback(&session, id, Rating::Positive, None, None)
            .expect("rating");
    }

    let ids: Vec<String> = store
        .all_feedback(&session)
        .expect("read all")
        .into_iter()
        .map(|entry| entry.message_id)
        .collect();

    assert_eq!(ids, vec!["msg-1", "msg-2", "msg-3"]);
}

/// The whole reason this is a sidecar: feedback must not reach the model.
#[test]
fn feedback_never_appears_in_the_message_log() {
    let (_dir, store, session) = store();
    store
        .save_message(
            &session,
            0,
            "{\"role\":\"assistant\",\"content\":\"hello\"}",
        )
        .expect("message");
    store
        .set_feedback(
            &session,
            "msg-1",
            Rating::Negative,
            Some("this was wrong"),
            None,
        )
        .expect("rating");

    let messages = store.load_messages(&session).expect("messages");

    assert_eq!(
        messages.len(),
        1,
        "the rating must not have added a message"
    );
    assert!(
        !messages[0].contains("this was wrong"),
        "the note reached the log, and therefore model context: {}",
        messages[0]
    );
}

/// Two sessions rating the same message id must not see each other.
#[test]
fn ratings_are_scoped_to_their_session() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = SessionStore::open(&dir.path().join("sessions.db")).expect("store");
    let one = store
        .create_session(&dir.path().to_string_lossy(), None, "claude-opus-5")
        .expect("session one")
        .id;
    let two = store
        .create_session(&dir.path().to_string_lossy(), None, "claude-opus-5")
        .expect("session two")
        .id;

    store
        .set_feedback(&one, "msg-1", Rating::Positive, None, None)
        .expect("rating");

    assert!(store.feedback(&two, "msg-1").expect("read").is_none());
    assert_eq!(store.all_feedback(&two).expect("read all").len(), 0);
}
