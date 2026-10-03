// Issue-250: the calls whose reuse reads their slot alone, never the
// history (`call_record_for_reuse`). Fixtures from `result_store_tests_history.rs`.

#[test]
fn remediation_work_keeps_its_slot() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    let id = "review-fix-1";
    let mut fix_call = call(id);
    fix_call.options.extra.insert(
        "remediation_contract".to_string(),
        serde_json::json!({ "round": 1 }),
    );
    let mut accepted = accepted_at(id, 1, "input-x", T1);
    accepted.call = fix_call.clone();
    store.save_call_record(&accepted).unwrap();
    let mut interrupted = interrupted_at(id, 2, "input-y", "paused", T2_END);
    interrupted.call = fix_call.clone();
    store.save_call_record(&interrupted).unwrap();
    let candidate = store
        .call_record_for_reuse(&fix_call, "input-x")
        .unwrap()
        .unwrap();
    assert!(!candidate.from_history);
    assert_eq!(candidate.record.attempt, 2);
}

#[test]
fn fan_out_and_host_command_calls_keep_their_slot() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    // A fan-out call: a later attempt may have rewritten its branch outcomes.
    let id = "review-map-1";
    store
        .save_call_record(&accepted_at(id, 1, "input-x", T1))
        .unwrap();
    store
        .save_call_record(&interrupted_at(id, 2, "input-y", "paused", T2_END))
        .unwrap();
    let outcome = WorkflowV2BranchOutcome {
        item_id: "item-1".to_string(),
        role: "critic".to_string(),
        status: WorkflowV2Status::Accepted,
        result: None,
        error: None,
        failure_kind: None,
        item_input_hash: Some("item-input-y".to_string()),
        completion_evidence: Vec::new(),
    };
    store.save_branch_outcome(id, &outcome).unwrap();
    let candidate = store
        .call_record_for_reuse(&call(id), "input-x")
        .unwrap()
        .unwrap();
    assert!(!candidate.from_history);
    assert!(
        store
            .last_accepted_call_record(id, "input-x")
            .unwrap()
            .is_some()
    );

    let host = "host-command-1";
    let mut host_call = call(host);
    host_call.method = WorkflowV2HostMethod::HostCommand;
    let mut accepted = accepted_at(host, 1, "input-x", T1);
    accepted.call = host_call.clone();
    store.save_call_record(&accepted).unwrap();
    let mut interrupted = interrupted_at(host, 2, "input-y", "paused", T2_END);
    interrupted.call = host_call.clone();
    store.save_call_record(&interrupted).unwrap();
    let candidate = store
        .call_record_for_reuse(&host_call, "input-x")
        .unwrap()
        .unwrap();
    assert!(!candidate.from_history);
}

#[test]
fn unreadable_archived_records_never_repeat_an_attempt_number() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    let id = "author-unreadable";
    store
        .save_call_record(&accepted_at(id, 1, "input-x", T1))
        .unwrap();
    store
        .save_call_record(&interrupted_at(id, 2, "input-y", "paused", T2_END))
        .unwrap();
    let stem = store
        .result_path(id)
        .file_stem()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let dir = temp.path().join("results").join("superseded");
    // Shaped like a record but no longer one (its status is unknown), and
    // a file that is not JSON at all.
    let broken = serde_json::json!({
        "call": { "id": id }, "attempt": 7, "input_hash": "input-x",
        "status": "no-such-status", "result": {}
    });
    std::fs::write(dir.join(format!("{stem}-9-9-0.json")), broken.to_string()).unwrap();
    std::fs::write(dir.join(format!("{stem}-9-9-1.json")), b"not json").unwrap();

    assert_eq!(store.next_attempt(id).unwrap(), 8);
    // A gap in the history: the accepted record is not answered from it.
    assert!(
        store
            .last_accepted_call_record(id, "input-x")
            .unwrap()
            .is_none()
    );
    let candidate = store
        .call_record_for_reuse(&call(id), "input-x")
        .unwrap()
        .unwrap();
    assert!(!candidate.from_history);
}
