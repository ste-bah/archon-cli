use std::fs;
use std::path::PathBuf;

use crate::storage::{SessionError, SessionStore};

fn temp_store() -> (PathBuf, SessionStore) {
    let dir = std::env::temp_dir()
        .join("archon-session-message-rollback")
        .join(uuid::Uuid::new_v4().to_string());
    fs::create_dir_all(&dir).expect("create temp directory");
    let store = SessionStore::open(&dir.join("sessions.db")).expect("open store");
    (dir, store)
}

#[test]
fn list_sessions_uses_three_queries_for_empty_and_populated_results() {
    let (dir, store) = temp_store();

    store.reset_list_query_count();
    assert!(store.list_sessions(10).expect("list empty").is_empty());
    assert_eq!(store.list_query_count(), 3);

    store
        .register_session("listed-session", "/listed", None, "test")
        .expect("register session");
    store.reset_list_query_count();
    assert_eq!(store.list_sessions(10).expect("list populated").len(), 1);
    assert_eq!(store.list_query_count(), 3);

    fs::remove_dir_all(dir).expect("remove temp directory");
}

#[test]
fn replace_messages_rolls_back_rows_and_count_when_post_write_step_fails() {
    let (dir, store) = temp_store();
    let session = store
        .register_session("rollback-session", "/rollback", None, "test")
        .expect("register session");
    store.save_message(&session.id, 0, "old-0").unwrap();
    store.save_message(&session.id, 1, "old-1").unwrap();
    store.save_message(&session.id, 2, "old-2").unwrap();

    store.fail_next_replace_after_rows_are_written();
    let error = store
        .replace_messages(&session.id, &["new-0".to_string()])
        .unwrap_err();

    assert!(matches!(error, SessionError::DbError(_)));
    assert_eq!(
        store.load_messages(&session.id).unwrap(),
        vec!["old-0", "old-1", "old-2"]
    );
    assert_eq!(store.get_session(&session.id).unwrap().message_count, 3);

    fs::remove_dir_all(dir).expect("remove temp directory");
}
