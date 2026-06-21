use archon_workflow::{
    WorkflowV2BranchOutcome, WorkflowV2CallExecution, WorkflowV2Evidence, WorkflowV2EvidenceKind,
    WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2Result, WorkflowV2ResultStore,
    WorkflowV2Runtime, WorkflowV2Status,
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

#[test]
fn resume_reuses_completed_results_without_invoking_executor() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    let runtime = WorkflowV2Runtime::new(store);
    let executions = vec![
        call("a", serde_json::json!({"n": 1}), &[]),
        call("b", serde_json::json!({"n": 2}), &["a"]),
    ];

    runtime
        .run_serial(&executions, |execution| {
            Ok(accepted(&format!("{} accepted", execution.call.id)))
        })
        .expect("first run");

    let summary = runtime
        .run_serial(&executions, |_| panic!("executor should not be called"))
        .expect("resume run");

    assert_eq!(summary.executed, 0);
    assert_eq!(summary.reused, 2);
    assert_eq!(summary.completed, 2);
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
fn changed_input_hash_invalidates_cached_result() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    let runtime = WorkflowV2Runtime::new(store.clone());
    let first = vec![call("a", serde_json::json!({"n": 1}), &[])];
    let second = vec![call("a", serde_json::json!({"n": 2}), &[])];

    runtime
        .run_serial(&first, |_| Ok(accepted("first")))
        .expect("first run");
    runtime
        .run_serial(&second, |_| Ok(accepted("second")))
        .expect("second run");

    let record = store
        .load_call_record("a")
        .expect("load record")
        .expect("record");
    assert_eq!(record.attempt, 2);
    assert_eq!(record.result.summary, "second");
}

#[test]
fn changed_upstream_input_reruns_downstream_even_when_downstream_input_is_same() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    let runtime = WorkflowV2Runtime::new(store.clone());
    let first = vec![
        call("a", serde_json::json!({"n": 1}), &[]),
        call("b", serde_json::json!({"stable": true}), &["a"]),
    ];
    let second = vec![
        call("a", serde_json::json!({"n": 2}), &[]),
        call("b", serde_json::json!({"stable": true}), &["a"]),
    ];

    runtime
        .run_serial(&first, |execution| {
            Ok(accepted(&format!("{} first", execution.call.id)))
        })
        .expect("first run");
    let summary = runtime
        .run_serial(&second, |execution| {
            Ok(accepted(&format!("{} second", execution.call.id)))
        })
        .expect("second run");

    assert_eq!(summary.reused, 0);
    assert_eq!(summary.executed, 2);
    assert_eq!(
        store
            .load_call_record("b")
            .expect("load")
            .expect("record")
            .result
            .summary,
        "b second"
    );
}

#[test]
fn restart_invalidates_downstream_dependents() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    let runtime = WorkflowV2Runtime::new(store.clone());
    let executions = vec![
        call("a", serde_json::json!({"n": 1}), &[]),
        call("b", serde_json::json!({"n": 2}), &["a"]),
        call("c", serde_json::json!({"n": 3}), &["b"]),
    ];

    runtime
        .run_serial(&executions, |execution| {
            Ok(accepted(&format!("{} first", execution.call.id)))
        })
        .expect("first run");
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
    let summary = runtime
        .run_serial(&executions, |execution| {
            Ok(accepted(&format!("{} rerun", execution.call.id)))
        })
        .expect("resume");

    assert_eq!(summary.reused, 1);
    assert_eq!(summary.executed, 2);
    assert_eq!(
        store
            .load_call_record("a")
            .expect("load")
            .expect("record")
            .result
            .summary,
        "a first"
    );
    assert_eq!(
        store
            .load_call_record("b")
            .expect("load")
            .expect("record")
            .result
            .summary,
        "b rerun"
    );
    assert_eq!(
        store
            .load_call_record("c")
            .expect("load")
            .expect("record")
            .result
            .summary,
        "c rerun"
    );
}
