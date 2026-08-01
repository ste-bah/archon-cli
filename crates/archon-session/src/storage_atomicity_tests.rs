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
fn list_sessions_uses_one_query_for_empty_and_populated_results() {
    let (dir, store) = temp_store();

    store.reset_query_count();
    assert!(store.list_sessions(10).expect("list empty").is_empty());
    assert_eq!(store.query_count(), 1);

    store
        .register_session("listed-session", "/listed", None, "test")
        .expect("register session");
    store
        .set_name("listed-session", "Listed")
        .expect("set session name");
    store
        .set_parent("listed-session", "parent-session")
        .expect("set session parent");
    store
        .register_session("empty-metadata-session", "/empty", None, "test")
        .expect("register empty metadata session");
    store
        .set_name("empty-metadata-session", "")
        .expect("set empty session name");
    store
        .set_parent("empty-metadata-session", "")
        .expect("set empty session parent");
    store.reset_query_count();
    let sessions = store.list_sessions(10).expect("list populated");
    assert_eq!(sessions.len(), 2);
    let listed = sessions
        .iter()
        .find(|session| session.id == "listed-session")
        .expect("listed session");
    assert_eq!(listed.name.as_deref(), Some("Listed"));
    assert_eq!(listed.parent_session_id.as_deref(), Some("parent-session"));
    let empty = sessions
        .iter()
        .find(|session| session.id == "empty-metadata-session")
        .expect("empty metadata session");
    assert_eq!(empty.name.as_deref(), Some(""));
    assert_eq!(empty.parent_session_id.as_deref(), Some(""));
    assert_eq!(store.query_count(), 1);

    // Close the store before deleting its directory: Windows refuses to remove
    // a directory that still has an open handle inside it, while Unix happily
    // unlinks open files.
    drop(store);
    fs::remove_dir_all(dir).expect("remove temp directory");
}

#[test]
fn delete_session_rolls_back_compaction_cleanup_on_failure() {
    let (dir, store) = temp_store();
    let session = store
        .register_session("delete-rollback", "/rollback", None, "test")
        .unwrap();
    store.save_message(&session.id, 0, "message").unwrap();
    let segment = store
        .close_compaction_segment(&session.id, 0, 0, &["source".into()])
        .unwrap();

    store.fail_next_delete_after_compaction_rows();
    assert!(store.delete_session(&session.id).is_err());

    assert!(store.get_session(&session.id).is_ok());
    assert_eq!(store.load_messages(&session.id).unwrap(), vec!["message"]);
    assert!(store.get_compaction_segment(&segment.id).unwrap().is_some());
    assert_eq!(
        store.load_compaction_segment_body(&segment.id).unwrap(),
        vec!["source"]
    );

    // Close the store before deleting its directory: Windows refuses to remove
    // a directory that still has an open handle inside it, while Unix happily
    // unlinks open files.
    drop(store);
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

    // Close the store before deleting its directory: Windows refuses to remove
    // a directory that still has an open handle inside it, while Unix happily
    // unlinks open files.
    drop(store);
    fs::remove_dir_all(dir).expect("remove temp directory");
}
