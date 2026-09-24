// A file this store did not write must not fail the callers that walk it.
//
// Every stage loads these records while preparing its fan-out, so one
// unreadable file here used to fail every later stage before it started.

fn scan_store(dir: &tempfile::TempDir) -> WorkflowV2ResultStore {
    WorkflowV2ResultStore::new(dir.path().join("v2"))
}

fn scan_outcome(item_id: &str) -> WorkflowV2BranchOutcome {
    WorkflowV2BranchOutcome {
        item_id: item_id.to_string(),
        role: "coder".to_string(),
        status: WorkflowV2Status::Accepted,
        result: Some(WorkflowV2Result::accepted(format!("{item_id} accepted"))),
        error: None,
        failure_kind: None,
        item_input_hash: Some(format!("hash-{item_id}")),
        completion_evidence: Vec::new(),
    }
}

fn write_foreign_file(store: &WorkflowV2ResultStore, relative: &str, body: &str) {
    let path = store.root().join(relative);
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(&path, body).expect("write foreign file");
}

/// The live sequence: a branch wrote a hand-authored note into a directory of
/// its own under `branches/`, and the note happens to share one field name
/// with a branch outcome. Loading must skip it and still return every real
/// outcome, so the stages after it keep running.
#[test]
fn a_foreign_json_file_under_branches_is_skipped_and_the_real_outcomes_still_load() {
    let dir = tempfile::tempdir().expect("tmp");
    let store = scan_store(&dir);
    store
        .save_branch_outcome("call-one", &scan_outcome("0"))
        .expect("save first");
    store
        .save_branch_outcome("call-two", &scan_outcome("0"))
        .expect("save second");
    write_foreign_file(
        &store,
        "branches/call-one-0/note.json",
        r#"{"id":"a-note","status":"accepted","files_changed":["src/lib.rs"]}"#,
    );

    let outcomes = store.load_branch_outcomes().expect("load outcomes");

    assert_eq!(outcomes.len(), 2, "{outcomes:#?}");
    assert!(
        store.load_branch_outcomes_for_call("call-one-0").is_ok(),
        "the foreign file's own directory must load as empty, not as an error"
    );
    assert!(
        store
            .load_branch_outcomes_for_call("call-one-0")
            .expect("load")
            .is_empty()
    );
}

/// The same containment for the call-record directory, which every resume and
/// every completion ledger walks.
#[test]
fn a_foreign_json_file_under_results_is_skipped_and_the_real_records_still_load() {
    let dir = tempfile::tempdir().expect("tmp");
    let store = scan_store(&dir);
    save_task_record(&store, "call-one", ["TASK-ALPHA-010"], []);
    write_foreign_file(&store, "results/scratch.json", r#"{"note":"not a record"}"#);
    write_foreign_file(&store, "results/truncated.json", "{\"call\": ");

    let records = store.load_call_records().expect("load records");

    assert_eq!(records.len(), 1, "{records:#?}");
    assert_eq!(records[0].call.id, "call-one");
}

/// Skipping must stay narrow. A document shaped like one of this store's own
/// records and still unreadable is corrupt state, not a foreign file: dropping
/// it would quietly change what the run believes it completed. It surfaces as
/// a typed state fault naming the file, which is also the signal the host uses
/// to tell "never started" apart from "attempted and failed".
#[test]
fn a_record_shaped_document_that_will_not_parse_is_reported_as_corrupt_state() {
    let dir = tempfile::tempdir().expect("tmp");
    let store = scan_store(&dir);
    store
        .save_branch_outcome("call-one", &scan_outcome("0"))
        .expect("save");
    write_foreign_file(
        &store,
        "branches/call-one/1.json",
        r#"{"item_id":"1","role":"coder","status":"not-a-status","result":null,"error":null}"#,
    );

    let error = store
        .load_branch_outcomes()
        .expect_err("a corrupt own record must not be skipped");

    assert!(
        matches!(&error, WorkflowError::StateCorrupt(detail) if detail.contains("1.json")),
        "{error:?}"
    );
}
