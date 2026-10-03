// Issue-254: every archived record of a call lives in that call's own
// directory, so a history lookup reads only the call's own records, never a
// listing of every call's archive. A flat archive left by an older build is
// moved into the per-call directories on first touch and still answers.

/// Where the archived records of `id` are kept.
fn history_dir_of(store: &WorkflowV2ResultStore, id: &str) -> std::path::PathBuf {
    let stem = store.result_path(id);
    let stem = stem.file_stem().unwrap().to_str().unwrap();
    store.root().join("results").join("history").join(stem)
}

fn file_count(dir: &std::path::Path) -> usize {
    std::fs::read_dir(dir).map_or(0, |entries| entries.flatten().count())
}

fn flat_archive(store: &WorkflowV2ResultStore) -> std::path::PathBuf {
    store.root().join("results").join("superseded")
}

#[test]
fn each_calls_archive_lives_in_its_own_directory() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    for attempt in 1..=6 {
        store
            .save_call_record(&accepted_at(
                "busy",
                attempt,
                &format!("input-{attempt}"),
                T1,
            ))
            .unwrap();
    }
    store
        .save_call_record(&accepted_at("quiet", 1, "input-q", T1))
        .unwrap();
    store
        .save_call_record(&interrupted_at("quiet", 2, "input-r", "paused", T2_END))
        .unwrap();

    assert_eq!(file_count(&history_dir_of(&store, "busy")), 5);
    assert_eq!(file_count(&history_dir_of(&store, "quiet")), 1);
    assert!(!flat_archive(&store).exists(), "no shared flat archive");
    // The history still answers, from the call's own directory.
    let candidate = store
        .call_record_for_reuse(&call("quiet"), "input-q")
        .unwrap()
        .unwrap();
    assert!(candidate.from_history);
    assert_eq!(candidate.record.attempt, 1);
    assert_eq!(store.next_attempt("busy").unwrap(), 7);
    assert_eq!(store.next_attempt("quiet").unwrap(), 3);
    // Restored into the slot, the interrupted attempt joins the history.
    store.restore_call_record(&candidate.record).unwrap();
    assert_eq!(file_count(&history_dir_of(&store, "quiet")), 2);
    assert_eq!(store.next_attempt("quiet").unwrap(), 3);
}

#[test]
fn a_flat_archive_left_by_an_older_build_is_migrated_on_first_touch() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    let id = "implementation-wave-1";
    // The older layout: the accepted attempt renamed into the flat archive
    // (`<slot stem>-<time>-<pid>-<seq>.json`), the interrupted one in the
    // slot, and a broken archived file of another call.
    let stem = store.result_path(id);
    let stem = stem.file_stem().unwrap().to_str().unwrap().to_string();
    let other = store.result_path("other-call");
    let other = other.file_stem().unwrap().to_str().unwrap().to_string();
    let flat = flat_archive(&store);
    std::fs::create_dir_all(&flat).unwrap();
    let accepted = accepted_at(id, 1, "input-x", T1);
    std::fs::write(
        flat.join(format!("{stem}-1-1-0.json")),
        serde_json::to_vec(&accepted).unwrap(),
    )
    .unwrap();
    std::fs::write(flat.join(format!("{other}-2-2-1.json")), b"not json").unwrap();
    store
        .save_call_record(&interrupted_at(id, 2, "input-y", "paused", T2_END))
        .unwrap();

    let candidate = store
        .call_record_for_reuse(&call(id), "input-x")
        .unwrap()
        .expect("the flat archive still answers");
    assert!(candidate.from_history);
    assert_eq!(candidate.record, accepted);

    assert!(!flat.exists(), "the flat archive is migrated and removed");
    assert_eq!(file_count(&history_dir_of(&store, id)), 1);
    // The unreadable file moved to its own call too, where it still marks a
    // gap in that call's history.
    assert_eq!(file_count(&history_dir_of(&store, "other-call")), 1);
    assert_eq!(store.next_attempt(id).unwrap(), 3);
}
