use std::fs;
use std::path::PathBuf;

use archon_session::storage::{SessionError, SessionStore};

fn temp_store() -> (PathBuf, SessionStore) {
    let dir = std::env::temp_dir()
        .join("archon-session-storage-atomicity")
        .join(uuid::Uuid::new_v4().to_string());
    fs::create_dir_all(&dir).expect("create temp directory");
    let store = SessionStore::open(&dir.join("sessions.db")).expect("open store");
    (dir, store)
}

fn save_messages(store: &SessionStore, session_id: &str, messages: &[&str]) {
    for (index, content) in messages.iter().enumerate() {
        store
            .save_message(session_id, index as u64, content)
            .expect("save message");
    }
}

#[test]
fn replace_messages_exactly_replaces_shorter_and_longer_lists() {
    let (dir, store) = temp_store();
    let session = store
        .register_session("replace-session", "/replace", None, "test")
        .expect("register session");
    save_messages(&store, &session.id, &["old-0", "old-1", "old-2", "old-3"]);

    let shorter = vec!["short-0".to_string(), "short-1".to_string()];
    store
        .replace_messages(&session.id, &shorter)
        .expect("replace with shorter list");
    assert_eq!(store.load_messages(&session.id).unwrap(), shorter);
    assert_eq!(store.get_session(&session.id).unwrap().message_count, 2);

    let longer = vec![
        "long-0".to_string(),
        "long-1".to_string(),
        "long-2".to_string(),
        "long-3".to_string(),
        "long-4".to_string(),
    ];
    store
        .replace_messages(&session.id, &longer)
        .expect("replace with longer list");
    assert_eq!(store.load_messages(&session.id).unwrap(), longer);
    assert_eq!(store.get_session(&session.id).unwrap().message_count, 5);

    // Close the store before deleting its directory: Windows refuses to remove
    // a directory that still has an open handle inside it, while Unix happily
    // unlinks open files. That is why this only ever failed on Windows.
    drop(store);
    fs::remove_dir_all(dir).expect("remove temp directory");
}

#[test]
fn replace_messages_refuses_empty_without_mutating_existing_messages() {
    let (dir, store) = temp_store();
    let session = store
        .register_session("empty-replace-session", "/replace", None, "test")
        .expect("register session");
    save_messages(&store, &session.id, &["old-0", "old-1"]);

    let error = store.replace_messages(&session.id, &[]).unwrap_err();
    assert!(matches!(error, SessionError::EmptyReplaceRefused));
    assert_eq!(
        store.load_messages(&session.id).unwrap(),
        vec!["old-0", "old-1"]
    );
    assert_eq!(store.get_session(&session.id).unwrap().message_count, 2);

    // Close the store before deleting its directory: Windows refuses to remove
    // a directory that still has an open handle inside it, while Unix happily
    // unlinks open files. That is why this only ever failed on Windows.
    drop(store);
    fs::remove_dir_all(dir).expect("remove temp directory");
}

#[test]
fn truncate_messages_after_overshoot_preserves_count_and_content() {
    let (dir, store) = temp_store();
    let session = store
        .register_session("overshoot-session", "/truncate", None, "test")
        .expect("register session");
    save_messages(&store, &session.id, &["zero", "one"]);

    store
        .truncate_messages_after(&session.id, 99)
        .expect("overshoot truncate succeeds");

    assert_eq!(
        store.load_messages(&session.id).unwrap(),
        vec!["zero", "one"]
    );
    assert_eq!(store.get_session(&session.id).unwrap().message_count, 2);

    // Close the store before deleting its directory: Windows refuses to remove
    // a directory that still has an open handle inside it, while Unix happily
    // unlinks open files. That is why this only ever failed on Windows.
    drop(store);
    fs::remove_dir_all(dir).expect("remove temp directory");
}

#[test]
fn truncate_messages_after_removes_only_the_suffix() {
    let (dir, store) = temp_store();
    let session = store
        .register_session("truncate-session", "/truncate", None, "test")
        .expect("register session");
    save_messages(&store, &session.id, &["zero", "one", "two", "three"]);

    store
        .truncate_messages_after(&session.id, 1)
        .expect("truncate succeeds");

    assert_eq!(
        store.load_messages(&session.id).unwrap(),
        vec!["zero", "one"]
    );
    assert_eq!(store.get_session(&session.id).unwrap().message_count, 2);

    // Close the store before deleting its directory: Windows refuses to remove
    // a directory that still has an open handle inside it, while Unix happily
    // unlinks open files. That is why this only ever failed on Windows.
    drop(store);
    fs::remove_dir_all(dir).expect("remove temp directory");
}

#[test]
fn list_sessions_includes_optional_names_and_parents() {
    let (dir, store) = temp_store();
    store
        .register_session("parent-session", "/parent", None, "test")
        .expect("register parent");
    store
        .register_session("named-child", "/child", None, "test")
        .expect("register child");
    store
        .register_session("plain-session", "/plain", None, "test")
        .expect("register plain session");
    store.set_name("named-child", "Child session").unwrap();
    store.set_parent("named-child", "parent-session").unwrap();

    let sessions = store.list_sessions(10).expect("list sessions");
    let child = sessions
        .iter()
        .find(|session| session.id == "named-child")
        .expect("child session");
    assert_eq!(child.name.as_deref(), Some("Child session"));
    assert_eq!(child.parent_session_id.as_deref(), Some("parent-session"));

    let plain = sessions
        .iter()
        .find(|session| session.id == "plain-session")
        .expect("plain session");
    assert_eq!(plain.name, None);
    assert_eq!(plain.parent_session_id, None);

    // Close the store before deleting its directory: Windows refuses to remove
    // a directory that still has an open handle inside it, while Unix happily
    // unlinks open files. That is why this only ever failed on Windows.
    drop(store);
    fs::remove_dir_all(dir).expect("remove temp directory");
}
