//! Acceptance coverage for per-message feedback (#193 Phase C).

use archon_session::feedback::{FeedbackError, Rating, message_digest};
use archon_session::storage::{SessionError, SessionStore};

/// Stand-in digest for the tests that are not about digest handling.
const DIGEST: &str = "0011223344556677";

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
        .set_feedback(
            &session,
            "msg-1",
            DIGEST,
            Rating::Positive,
            Some("useful"),
            None,
        )
        .expect("first rating");
    assert_eq!(set.rating, Rating::Positive);
    assert_eq!(set.note.as_deref(), Some("useful"));

    let changed = store
        .set_feedback(
            &session,
            "msg-1",
            DIGEST,
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
        .set_feedback(&session, "msg-1", DIGEST, Rating::Positive, None, None)
        .expect("first");

    // A second window wrote while the first was still holding its version.
    store
        .set_feedback(
            &session,
            "msg-1",
            DIGEST,
            Rating::Negative,
            Some("changed my mind"),
            Some(&first.version),
        )
        .expect("second window");

    let error = store
        .set_feedback(
            &session,
            "msg-1",
            DIGEST,
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
        .set_feedback(&session, "msg-1", DIGEST, Rating::Positive, None, None)
        .expect("first");

    let error = store
        .set_feedback(&session, "msg-1", DIGEST, Rating::Negative, None, None)
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
        .set_feedback(&session, "msg-1", DIGEST, Rating::Positive, None, None)
        .expect("first");
    store
        .clear_feedback(&session, "msg-1", Some(&set.version))
        .expect("clear");

    let error = store
        .set_feedback(
            &session,
            "msg-1",
            DIGEST,
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
            .set_feedback(&session, id, DIGEST, Rating::Positive, None, None)
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
            DIGEST,
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
        .set_feedback(&one, "msg-1", DIGEST, Rating::Positive, None, None)
        .expect("rating");

    assert!(store.feedback(&two, "msg-1").expect("read").is_none());
    assert_eq!(store.all_feedback(&two).expect("read all").len(), 0);
}

// ---------------------------------------------------------------------------
// Index drift (#193 Phase C, corrected)
// ---------------------------------------------------------------------------

/// A message has no id of its own, so a rating is keyed by its position — and
/// positions move. Compaction replaces the whole message list with a shorter
/// one, so index 7 afterwards is a different message from index 7 before, and
/// a rating left keyed to 7 would be reported as describing text nobody rated.
///
/// The digest is what makes that detectable. This is the property the reader
/// relies on; `build_feedback_snapshot` is where it is acted on.
#[test]
fn a_rating_records_which_message_it_was_about() {
    let (_dir, store, session) = store();
    let rated = r#"{"role":"assistant","content":"the original answer"}"#;
    let after_compaction = r#"{"role":"assistant","content":"something else entirely"}"#;

    store
        .set_feedback(
            &session,
            "7",
            &message_digest(rated),
            Rating::Positive,
            None,
            None,
        )
        .expect("rating");

    let found = store
        .feedback(&session, "7")
        .expect("read")
        .expect("still there");

    assert_eq!(
        found.message_digest,
        message_digest(rated),
        "the rating must say which message it was about"
    );
    assert_ne!(
        found.message_digest,
        message_digest(after_compaction),
        "a different message at the same index must not match"
    );
}

/// Identical text hashes identically, so re-rating the same message after a
/// harmless rewrite is not treated as a different message.
#[test]
fn the_digest_is_of_the_content_not_the_position() {
    let content = r#"{"role":"assistant","content":"same bytes"}"#;
    assert_eq!(message_digest(content), message_digest(content));
    assert_ne!(message_digest(content), message_digest("same byte"));
}

/// The stale row must stay overwritable. A reader that ignores a mismatched
/// rating still holds its version, and re-rating has to succeed rather than
/// dead-end on a conflict nobody can resolve.
#[test]
fn a_rating_about_a_different_message_can_be_overwritten_in_place() {
    let (_dir, store, session) = store();
    let old = store
        .set_feedback(
            &session,
            "7",
            &message_digest("old message"),
            Rating::Positive,
            None,
            None,
        )
        .expect("first rating");

    let replaced = store
        .set_feedback(
            &session,
            "7",
            &message_digest("what is there now"),
            Rating::Negative,
            Some("rating the message that is actually here"),
            Some(&old.version),
        )
        .expect("re-rating a shifted index must succeed");

    assert_eq!(replaced.rating, Rating::Negative);
    assert_eq!(replaced.message_digest, message_digest("what is there now"));
    assert_eq!(
        replaced.created_at, old.created_at,
        "the row is the same row"
    );
}

/// The defect this was shipped with, and the reason live verification exists.
///
/// `create_relation` treats "already exists" as success, so adding
/// `message_digest` changed the code and not any database that had already
/// created the relation. Opening such a store used to succeed and then fail on
/// every write with "stored relation 'message_feedback' does not have field
/// 'message_digest'" — a schema divergence that only surfaced against a real
/// session store.
#[test]
fn a_store_created_before_the_digest_field_is_rebuilt_rather_than_left_broken() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("sessions.db");

    // A database carrying the pre-digest shape of the relation.
    {
        let store = SessionStore::open(&path).expect("store");
        store
            .db()
            .run_mutable(
                "::remove message_feedback",
                std::collections::BTreeMap::new(),
                "test: drop the current relation",
            )
            .expect("drop");
        store
            .db()
            .run_mutable(
                ":create message_feedback {
                    session_id: String, message_id: String =>
                    rating: String, note: String, version: String,
                    created_at: String, updated_at: String
                }",
                std::collections::BTreeMap::new(),
                "test: recreate the old shape",
            )
            .expect("create old shape");
    }

    // Reopening must notice and rebuild, not carry on and fail on first write.
    let store = SessionStore::open(&path).expect("reopen");
    let session = store
        .create_session(&dir.path().to_string_lossy(), None, "claude-opus-5")
        .expect("session")
        .id;

    store
        .set_feedback(&session, "0", DIGEST, Rating::Positive, None, None)
        .expect("a rating must be writable after the rebuild");
    assert_eq!(
        store
            .feedback(&session, "0")
            .expect("read")
            .expect("present")
            .message_digest,
        DIGEST
    );
}
