use super::*;

fn append_tail(tail: &str, expected: &[&str]) {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path());
    fs::create_dir_all(store.run_dir("run")).unwrap();
    fs::write(store.events_path("run"), tail).unwrap();
    store.append_event_line("run", "{\"second\":true}").unwrap();
    let raw = fs::read_to_string(store.events_path("run")).unwrap();
    assert_eq!(raw.lines().collect::<Vec<_>>(), expected);
    serde_json::from_str::<serde_json::Value>(raw.lines().last().unwrap()).unwrap();
}

#[test]
fn append_after_valid_unterminated_event() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path());
    fs::create_dir_all(store.run_dir("run")).unwrap();
    let log = crate::WorkflowEventLog::new(store.clone());
    let first = log
        .emit(
            "run",
            1,
            crate::WorkflowEventKind::StageStarted,
            serde_json::json!({}),
        )
        .unwrap();
    fs::write(
        store.events_path("run"),
        serde_json::to_vec(&first).unwrap(),
    )
    .unwrap();
    log.emit(
        "run",
        2,
        crate::WorkflowEventKind::StageCompleted,
        serde_json::json!({}),
    )
    .unwrap();
    let raw = fs::read_to_string(store.events_path("run")).unwrap();
    let events: Vec<crate::WorkflowEvent> = raw
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].seq, 1);
    assert_eq!(events[1].seq, 2);
}

#[test]
fn append_after_partial_only_tail() {
    append_tail("{\"first\":", &["{\"first\":", "{\"second\":true}"]);
}

#[test]
fn append_to_empty_log() {
    append_tail("", &["{\"second\":true}"]);
}

#[test]
fn sequence_count_survives_split_utf8_tail() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path());
    fs::create_dir_all(store.run_dir("run")).unwrap();
    fs::write(store.events_path("run"), b"{}\n{\"detail\":\"\xc3").unwrap();
    assert_eq!(store.next_event_seq("run").unwrap(), 3);
}

/// Issue 318: secret-bearing records publish through `rename_durable`, like
/// every other durable write. A plain rename may return on Windows before
/// the move is on disk; on Unix the two are the same call, so this guards
/// the source rather than a crash.
#[test]
fn private_records_publish_through_the_durable_rename() {
    let source = include_str!("store_private.rs");
    assert!(!source.contains("fs::rename("), "{source}");
    assert!(source.contains("rename_durable(&tmp, &target)"), "{source}");
}
