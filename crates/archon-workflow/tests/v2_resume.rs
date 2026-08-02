use archon_workflow::{
    WorkflowV2BranchOutcome, WorkflowV2CallExecution, WorkflowV2CallRecord, WorkflowV2Evidence,
    WorkflowV2EvidenceKind, WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2Result,
    WorkflowV2ResultStore, WorkflowV2Status,
};

fn call(id: &str, input: serde_json::Value, depends_on: &[&str]) -> WorkflowV2CallExecution {
    WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            id: id.to_string(),
            method: WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: Default::default(),
        },
        input,
        depends_on: depends_on.iter().map(|id| id.to_string()).collect(),
    }
}

fn accepted(summary: &str) -> WorkflowV2Result {
    let mut result = WorkflowV2Result::accepted(summary);
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Inspection,
        "checked durable typed evidence",
    ));
    result
}

/// Record a completed call the way a live scheduler would, so the store is in a
/// realistic resumable state before invalidation is exercised.
fn record_completed(
    store: &WorkflowV2ResultStore,
    execution: &WorkflowV2CallExecution,
    summary: &str,
) {
    let record = WorkflowV2CallRecord::new(
        store.run_id(),
        execution.call.clone(),
        1,
        format!("hash-{}", execution.call.id),
        accepted(summary),
        execution.depends_on.clone(),
    );
    store.save_call_record(&record).expect("save call record");
    let mut checkpoint = store
        .load_checkpoint()
        .expect("load checkpoint")
        .unwrap_or_default();
    checkpoint.mark_completed(&execution.call.id);
    store.save_checkpoint(&checkpoint).expect("save checkpoint");
}

#[test]
fn result_store_paths_do_not_collide_after_sanitizing_call_ids() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());

    assert_ne!(store.result_path("a/b"), store.result_path("a_b"));
}

#[test]
fn branch_outcomes_are_persisted_to_per_call_item_artifacts() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    let outcome = WorkflowV2BranchOutcome {
        item_id: "TASK/T001".to_string(),
        role: "critic".to_string(),
        status: WorkflowV2Status::Accepted,
        result: Some(accepted("branch accepted")),
        error: None,
        failure_kind: None,
        item_input_hash: Some("test-input-hash-task-t001".to_string()),
        completion_evidence: Vec::new(),
    };

    let path = store
        .save_branch_outcome("review/wave", &outcome)
        .expect("save branch outcome");

    assert!(path.exists());
    assert_eq!(path, store.branch_outcome_path("review/wave", "TASK/T001"));
    let raw = std::fs::read_to_string(path).expect("read branch outcome");
    assert!(raw.contains("TASK/T001"));
    assert!(raw.contains("branch accepted"));
}

#[test]
fn restart_invalidates_downstream_dependents() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    let executions = vec![
        call("a", serde_json::json!({"n": 1}), &[]),
        call("b", serde_json::json!({"n": 2}), &["a"]),
        call("c", serde_json::json!({"n": 3}), &["b"]),
    ];
    for execution in &executions {
        record_completed(&store, execution, &format!("{} first", execution.call.id));
    }

    let invalidated = store
        .invalidate_call_and_dependents(&executions, "b")
        .expect("invalidate");

    assert_eq!(invalidated, vec!["b", "c"]);
    assert_eq!(
        store
            .load_checkpoint()
            .expect("load checkpoint")
            .expect("checkpoint")
            .completed_call_ids,
        vec!["a"]
    );
    // The untouched upstream call keeps its cached result.
    assert_eq!(
        store
            .load_call_record("a")
            .expect("load")
            .expect("record")
            .result
            .summary,
        "a first"
    );
}
