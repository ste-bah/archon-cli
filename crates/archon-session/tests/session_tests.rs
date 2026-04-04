use std::fs;
use std::path::PathBuf;

use archon_session::resume::{format_session_line, resume_session};
use archon_session::storage::SessionStore;

fn temp_db() -> (PathBuf, SessionStore) {
    let dir = std::env::temp_dir()
        .join("archon-session-test")
        .join(uuid::Uuid::new_v4().to_string());
    fs::create_dir_all(&dir).expect("create dir");
    let path = dir.join("test.db");
    let store = SessionStore::open(&path).expect("open db");
    (dir, store)
}

fn cleanup(dir: &PathBuf) {
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn wal_mode_active() {
    let (dir, store) = temp_db();
    assert!(store.verify_wal_mode().expect("check WAL"), "WAL mode should be active");
    cleanup(&dir);
}

#[test]
fn create_session() {
    let (dir, store) = temp_db();
    let meta = store
        .create_session("/home/user/project", Some("main"), "claude-sonnet-4-6")
        .expect("create session");

    assert!(!meta.id.is_empty());
    assert_eq!(meta.working_directory, "/home/user/project");
    assert_eq!(meta.git_branch.as_deref(), Some("main"));
    assert_eq!(meta.model, "claude-sonnet-4-6");
    assert_eq!(meta.message_count, 0);
    assert_eq!(meta.schema_version, 1);

    cleanup(&dir);
}

#[test]
fn save_and_load_messages() {
    let (dir, store) = temp_db();
    let meta = store
        .create_session("/tmp", None, "opus")
        .expect("create");

    store
        .save_message(&meta.id, 0, r#"{"role":"user","content":"hello"}"#)
        .expect("save msg 0");
    store
        .save_message(&meta.id, 1, r#"{"role":"assistant","content":"hi"}"#)
        .expect("save msg 1");

    let messages = store.load_messages(&meta.id).expect("load messages");
    assert_eq!(messages.len(), 2);
    assert!(messages[0].contains("hello"));
    assert!(messages[1].contains("hi"));

    // Verify message count updated
    let updated = store.get_session(&meta.id).expect("get session");
    assert_eq!(updated.message_count, 2);

    cleanup(&dir);
}

#[test]
fn list_sessions_sorted_by_last_active() {
    let (dir, store) = temp_db();

    store.create_session("/first", None, "model").expect("s1");
    std::thread::sleep(std::time::Duration::from_millis(10));
    store.create_session("/second", None, "model").expect("s2");
    std::thread::sleep(std::time::Duration::from_millis(10));
    store.create_session("/third", None, "model").expect("s3");

    let sessions = store.list_sessions(20).expect("list");
    assert_eq!(sessions.len(), 3);
    // Most recent first
    assert_eq!(sessions[0].working_directory, "/third");
    assert_eq!(sessions[2].working_directory, "/first");

    cleanup(&dir);
}

#[test]
fn list_sessions_respects_limit() {
    let (dir, store) = temp_db();

    for i in 0..10 {
        store
            .create_session(&format!("/dir{i}"), None, "m")
            .expect("create");
    }

    let sessions = store.list_sessions(3).expect("list");
    assert_eq!(sessions.len(), 3);

    cleanup(&dir);
}

#[test]
fn get_nonexistent_session() {
    let (dir, store) = temp_db();
    let result = store.get_session("nonexistent-id");
    assert!(result.is_err());
    cleanup(&dir);
}

#[test]
fn update_usage() {
    let (dir, store) = temp_db();
    let meta = store.create_session("/tmp", None, "m").expect("create");

    store.update_usage(&meta.id, 5000, 0.15).expect("update usage");

    let updated = store.get_session(&meta.id).expect("get");
    assert_eq!(updated.total_tokens, 5000);
    assert!((updated.total_cost - 0.15).abs() < f64::EPSILON);

    cleanup(&dir);
}

#[test]
fn resume_session_loads_messages() {
    let (dir, store) = temp_db();
    let meta = store.create_session("/project", Some("dev"), "opus").expect("create");

    store.save_message(&meta.id, 0, "msg-0").expect("save 0");
    store.save_message(&meta.id, 1, "msg-1").expect("save 1");
    store.save_message(&meta.id, 2, "msg-2").expect("save 2");

    let (resumed_meta, messages) = resume_session(&store, &meta.id).expect("resume");
    assert_eq!(resumed_meta.id, meta.id);
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0], "msg-0");
    assert_eq!(messages[2], "msg-2");

    cleanup(&dir);
}

#[test]
fn format_session_line_output() {
    let (dir, store) = temp_db();
    let meta = store
        .create_session("/home/user/project", Some("main"), "sonnet")
        .expect("create");

    let line = format_session_line(&meta);
    assert!(line.contains(&meta.id[..8]));
    assert!(line.contains("/home/user/project"));
    assert!(line.contains("main"));

    cleanup(&dir);
}

#[test]
fn database_persists_across_reopens() {
    let dir = std::env::temp_dir()
        .join("archon-session-test")
        .join(uuid::Uuid::new_v4().to_string());
    fs::create_dir_all(&dir).expect("create dir");
    let path = dir.join("persist.db");

    let session_id;
    {
        let store = SessionStore::open(&path).expect("open 1");
        let meta = store.create_session("/tmp", None, "m").expect("create");
        session_id = meta.id;
        store.save_message(&session_id, 0, "persisted").expect("save");
    }

    // Reopen
    {
        let store = SessionStore::open(&path).expect("open 2");
        let messages = store.load_messages(&session_id).expect("load");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0], "persisted");
    }

    let _ = fs::remove_dir_all(&dir);
}
