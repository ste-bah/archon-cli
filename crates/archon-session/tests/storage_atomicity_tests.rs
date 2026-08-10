use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use archon_session::storage::{SessionError, SessionStore};
use cozo::{DataValue, ScriptMutability};

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

/// Write a `messages` row straight into Cozo, bypassing `save_message`.
///
/// This is how the pre-fix persistence path left the database: it upserted
/// the indexes it currently held and never removed the physical tail. The
/// public API has no way to produce that state any more — which is exactly
/// why these two regressions need raw row injection to stay honest.
fn put_message_row(store: &SessionStore, session_id: &str, index: i64, content: &str) {
    let mut params = BTreeMap::new();
    params.insert("session_id".to_string(), DataValue::from(session_id));
    params.insert("message_index".to_string(), DataValue::from(index));
    params.insert("content".to_string(), DataValue::from(content));
    store
        .db()
        .run_mutable(
            "?[session_id, message_index, content] <- [[$session_id, $message_index, $content]]
             :put messages {session_id, message_index => content}",
            params,
            "test: inject stale message row",
        )
        .expect("inject stale message row");
}

/// Number of physical `messages` rows for a session, ignoring `message_count`.
fn physical_row_count(store: &SessionStore, session_id: &str) -> usize {
    let mut params = BTreeMap::new();
    params.insert("sid".to_string(), DataValue::from(session_id));
    store
        .db()
        .run_script(
            "?[message_index] := *messages{session_id, message_index}, session_id = $sid",
            params,
            ScriptMutability::Immutable,
        )
        .expect("count physical message rows")
        .rows
        .len()
}

/// Overwrite the logical `message_count` without touching any message row.
fn force_logical_message_count(store: &SessionStore, session_id: &str, count: i64) {
    let session = store.get_session(session_id).expect("session metadata");
    let mut params = BTreeMap::new();
    params.insert("id".to_string(), DataValue::from(session.id.as_str()));
    params.insert(
        "created_at".to_string(),
        DataValue::from(session.created_at.as_str()),
    );
    params.insert(
        "last_active".to_string(),
        DataValue::from(session.last_active.as_str()),
    );
    params.insert(
        "working_directory".to_string(),
        DataValue::from(session.working_directory.as_str()),
    );
    params.insert(
        "git_branch".to_string(),
        DataValue::from(session.git_branch.as_deref().unwrap_or("")),
    );
    params.insert("model".to_string(), DataValue::from(session.model.as_str()));
    params.insert("message_count".to_string(), DataValue::from(count));
    params.insert(
        "total_tokens".to_string(),
        DataValue::from(session.total_tokens as i64),
    );
    params.insert(
        "total_cost".to_string(),
        DataValue::from(session.total_cost),
    );
    params.insert(
        "schema_version".to_string(),
        DataValue::from(i64::from(session.schema_version)),
    );
    store
        .db()
        .run_mutable(
            "?[id, created_at, last_active, working_directory, git_branch, model, message_count, total_tokens, total_cost, schema_version] <- [[
                $id, $created_at, $last_active, $working_directory, $git_branch, $model, $message_count, $total_tokens, $total_cost, $schema_version
             ]]
             :put sessions {id => created_at, last_active, working_directory, git_branch, model, message_count, total_tokens, total_cost, schema_version}",
            params,
            "test: force logical message count",
        )
        .expect("force logical message count");
}

/// Issue #37 AC#3, read half: `load_messages` is bounded by the logical
/// `message_count`, not by whatever rows happen to be on disk.
///
/// Added by c00e48a6d as an in-crate test that reached the private
/// `set_message_count`; dropped by the a3ced0d5e refactor. Restored here
/// against the public surface, with the divergence between physical rows
/// and logical count injected directly.
#[test]
fn load_messages_clamps_physical_rows_above_logical_count() {
    let (dir, store) = temp_store();
    let session = store
        .register_session("clamp-session", "/clamp", None, "test")
        .expect("register session");
    save_messages(&store, &session.id, &["m-0", "m-1", "m-2", "m-3", "m-4"]);

    // Compaction shrank the conversation to two messages, but the pre-fix
    // writer only updated the count — rows 2..4 are still on disk.
    force_logical_message_count(&store, &session.id, 2);
    assert_eq!(
        physical_row_count(&store, &session.id),
        5,
        "fixture precondition: the stale tail must still be present"
    );

    assert_eq!(
        store.load_messages(&session.id).unwrap(),
        vec!["m-0", "m-1"],
        "load_messages must clamp to message_count and drop the stale tail"
    );

    // Close the store before deleting its directory: Windows refuses to remove
    // a directory that still has an open handle inside it, while Unix happily
    // unlinks open files.
    drop(store);
    fs::remove_dir_all(dir).expect("remove temp directory");
}

/// Issue #37 AC#3, write half: after `/resume` the next persistence pass
/// re-saves what `load_messages` returned, and that re-save must delete the
/// stale tail rather than leave it addressable.
///
/// Added by c00e48a6d, dropped by a3ced0d5e. Restored with an explicit
/// reopen so the "and restart" clause of the acceptance criterion is
/// actually crossed, and with a stale row planted past the compacted
/// length so the round trip has something to fail on.
#[test]
fn post_resume_replacement_does_not_resurrect_stale_tail() {
    let (dir, store) = temp_store();
    let session = store
        .register_session("resume-tail-session", "/resume-tail", None, "test")
        .expect("register session");
    save_messages(
        &store,
        &session.id,
        &[
            "old-0", "old-1", "old-2", "old-3", "old-4", "old-5", "old-6", "old-7", "old-8",
            "old-9",
        ],
    );

    let compacted: Vec<String> = (0..4).map(|i| format!("compact-{i}")).collect();
    store
        .replace_messages(&session.id, &compacted)
        .expect("replace with compacted messages");
    // A row a buggy writer could have left behind after the compaction.
    put_message_row(&store, &session.id, 7, "stale-7");

    // Restart: drop the handle and reopen the same database file.
    drop(store);
    let store = SessionStore::open(&dir.join("sessions.db")).expect("reopen store");

    let resumed = store
        .load_messages(&session.id)
        .expect("load after restart");
    assert_eq!(
        resumed, compacted,
        "resume must see only the compacted message set"
    );

    // The next turn re-saves exactly what resume handed back.
    store
        .replace_messages(&session.id, &resumed)
        .expect("re-save resumed messages");

    assert_eq!(store.load_messages(&session.id).unwrap(), compacted);
    assert_eq!(store.get_session(&session.id).unwrap().message_count, 4);
    assert_eq!(
        physical_row_count(&store, &session.id),
        4,
        "the re-save must physically remove the stale tail, not just hide it"
    );

    // Close the store before deleting its directory: Windows refuses to remove
    // a directory that still has an open handle inside it, while Unix happily
    // unlinks open files.
    drop(store);
    fs::remove_dir_all(dir).expect("remove temp directory");
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
