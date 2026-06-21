use archon_workflow::{
    WorkflowV2CallExecution, WorkflowV2CallRecord, WorkflowV2Checkpoint, WorkflowV2CommandKind,
    WorkflowV2CommandRecord, WorkflowV2CommandStatus, WorkflowV2Evidence, WorkflowV2EvidenceKind,
    WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions, WorkflowV2Result,
    WorkflowV2ResultStore, WorkflowV2Runtime, WorkflowV2Status,
};

fn call(id: &str) -> WorkflowV2CallExecution {
    WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            id: id.to_string(),
            method: WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: Default::default(),
        },
        input: serde_json::json!({ "call": id }),
        depends_on: Vec::new(),
    }
}

fn accepted_result(summary: &str) -> WorkflowV2Result {
    let mut result = WorkflowV2Result::accepted(summary);
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Inspection,
        "inspected concrete input",
    ));
    result
}

fn failed_result(summary: &str) -> WorkflowV2Result {
    WorkflowV2Result {
        status: WorkflowV2Status::Failed,
        summary: summary.to_string(),
        evidence: vec![WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Review,
            "terminal failure evidence",
        )],
        artifacts: Vec::new(),
        commands_run: vec![WorkflowV2CommandRecord {
            kind: WorkflowV2CommandKind::Test,
            command: "cargo test focused_case".to_string(),
            status: WorkflowV2CommandStatus::Failed,
            exit_code: Some(101),
            output_summary: "focused test failed".to_string(),
        }],
        files_read: Vec::new(),
        files_changed: Vec::new(),
        task_coverage: Vec::new(),
        residual_gaps: Vec::new(),
        data: serde_json::Value::Null,
    }
}

fn review_result(summary: &str) -> WorkflowV2Result {
    WorkflowV2Result {
        status: WorkflowV2Status::NeedsReview,
        summary: summary.to_string(),
        evidence: vec![WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Review,
            "non-terminal finding carried forward as workflow data",
        )],
        artifacts: Vec::new(),
        commands_run: Vec::new(),
        files_read: Vec::new(),
        files_changed: Vec::new(),
        task_coverage: Vec::new(),
        residual_gaps: Vec::new(),
        data: serde_json::json!({ "items": [] }),
    }
}

#[test]
fn fake_harness_completes_and_checkpoints_each_call() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    let runtime = WorkflowV2Runtime::new(store.clone());
    let executions = vec![call("inspect"), call("implement"), call("final")];

    let summary = runtime
        .run_serial(&executions, |execution| {
            Ok(accepted_result(&format!("{} accepted", execution.call.id)))
        })
        .expect("run");

    assert_eq!(summary.status, WorkflowV2Status::Accepted);
    assert_eq!(summary.completed, 3);
    assert_eq!(summary.executed, 3);
    assert_eq!(summary.reused, 0);
    let checkpoint = store
        .load_checkpoint()
        .expect("load checkpoint")
        .expect("checkpoint");
    assert_eq!(
        checkpoint.completed_call_ids,
        vec!["inspect", "implement", "final"]
    );
    for id in ["inspect", "implement", "final"] {
        let record = store
            .load_call_record(id)
            .expect("load record")
            .expect("record");
        assert_eq!(record.status, WorkflowV2Status::Accepted);
        assert_eq!(record.attempt, 1);
        assert_eq!(record.schema_version, "workflow-result-v2");
        assert_eq!(record.run_id, store.run_id());
        assert!(!record.started_at.is_empty());
        assert!(!record.finished_at.is_empty());
        assert!(!record.output_hash.is_empty());
    }
}

#[test]
fn failed_terminal_run_reports_next_action_and_stops() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    let runtime = WorkflowV2Runtime::new(store.clone());
    let executions = vec![call("inspect"), call("verify"), call("final")];

    let summary = runtime
        .run_serial(&executions, |execution| {
            if execution.call.id == "verify" {
                Ok(failed_result("verification failed"))
            } else {
                Ok(accepted_result(&format!("{} accepted", execution.call.id)))
            }
        })
        .expect("run");

    assert_eq!(summary.status, WorkflowV2Status::Failed);
    assert_eq!(summary.failed_call.as_deref(), Some("verify"));
    assert!(summary.next_action.as_deref().unwrap().contains("verify"));
    assert!(store.load_call_record("final").expect("load").is_none());
    let checkpoint = store
        .load_checkpoint()
        .expect("load checkpoint")
        .expect("checkpoint");
    assert_eq!(checkpoint.completed_call_ids, vec!["inspect"]);
}

#[test]
fn non_terminal_review_result_is_checkpointed_and_does_not_stop_downstream_calls() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    let runtime = WorkflowV2Runtime::new(store.clone());
    let executions = vec![call("audit"), call("reduce"), call("final")];

    let summary = runtime
        .run_serial(&executions, |execution| {
            if execution.call.id == "audit" {
                Ok(review_result("audit found remediation input"))
            } else {
                Ok(accepted_result(&format!("{} accepted", execution.call.id)))
            }
        })
        .expect("run");

    assert_eq!(summary.status, WorkflowV2Status::Accepted);
    assert_eq!(summary.completed, 3);
    assert_eq!(summary.failed_call, None);
    assert!(store.load_call_record("reduce").expect("load").is_some());
    let checkpoint = store
        .load_checkpoint()
        .expect("load checkpoint")
        .expect("checkpoint");
    assert_eq!(
        checkpoint.completed_call_ids,
        vec!["audit", "reduce", "final"]
    );
}

#[test]
fn terminal_failed_result_is_not_reused_on_resume() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    let runtime = WorkflowV2Runtime::new(store.clone());
    let executions = vec![call("inspect"), call("verify"), call("final")];

    runtime
        .run_serial(&executions, |execution| {
            if execution.call.id == "verify" {
                Ok(failed_result("verification failed"))
            } else {
                Ok(accepted_result(&format!("{} accepted", execution.call.id)))
            }
        })
        .expect("first run");

    let summary = runtime
        .run_serial(&executions, |execution| {
            Ok(accepted_result(&format!(
                "{} accepted on resume",
                execution.call.id
            )))
        })
        .expect("resume run");

    assert_eq!(summary.status, WorkflowV2Status::Accepted);
    assert_eq!(summary.reused, 1);
    assert_eq!(summary.executed, 2);
    assert_eq!(
        store
            .load_call_record("inspect")
            .expect("load inspect")
            .expect("inspect record")
            .attempt,
        1
    );
    let verify = store
        .load_call_record("verify")
        .expect("load verify")
        .expect("verify record");
    assert_eq!(verify.attempt, 2);
    assert_eq!(verify.status, WorkflowV2Status::Accepted);
    assert_eq!(verify.result.summary, "verify accepted on resume");
}

#[test]
fn failed_terminal_run_removes_stale_checkpoint_entry() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    let runtime = WorkflowV2Runtime::new(store.clone());
    let executions = vec![call("inspect"), call("verify"), call("final")];
    let mut checkpoint = WorkflowV2Checkpoint::default();
    checkpoint.mark_completed("inspect");
    checkpoint.mark_completed("verify");
    store
        .save_checkpoint(&checkpoint)
        .expect("seed stale checkpoint");

    let summary = runtime
        .run_serial(&executions, |execution| {
            if execution.call.id == "verify" {
                Ok(failed_result("verification failed"))
            } else {
                Ok(accepted_result(&format!("{} accepted", execution.call.id)))
            }
        })
        .expect("run");

    assert_eq!(summary.status, WorkflowV2Status::Failed);
    let checkpoint = store
        .load_checkpoint()
        .expect("load checkpoint")
        .expect("checkpoint");
    assert_eq!(checkpoint.completed_call_ids, vec!["inspect"]);
}

#[tokio::test]
async fn prepared_source_data_changes_invalidate_downstream_cache() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path());
    let runtime = WorkflowV2Runtime::new(store.clone());
    let downstream = WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            id: "downstream".to_string(),
            method: WorkflowV2HostMethod::Reduce,
            write_mode: None,
            options: WorkflowV2HostOptions {
                source: Some("source.items".to_string()),
                ..WorkflowV2HostOptions::default()
            },
        },
        input: serde_json::json!({ "call": "downstream" }),
        depends_on: vec!["source".to_string()],
    };
    let executions = vec![call("source"), downstream];

    let first = runtime
        .run_serial_async_with_prepared_inputs(
            &executions,
            {
                let store = store.clone();
                move |execution| {
                    let store = store.clone();
                    async move { Ok(prepared_source_execution(execution, &store)) }
                }
            },
            |execution| async move {
                if execution.call.id == "source" {
                    let mut result = accepted_result("source accepted");
                    result.data = serde_json::json!({ "items": [{ "id": "a" }] });
                    Ok(result)
                } else {
                    let mut result = accepted_result("downstream accepted");
                    result.data = serde_json::json!({
                        "observed_source": execution.input.get("source_data").cloned()
                    });
                    Ok(result)
                }
            },
        )
        .await
        .expect("first run");
    assert_eq!(first.executed, 2);

    let source_record = store
        .load_call_record("source")
        .expect("load source")
        .expect("source record");
    let mut changed_source = accepted_result("source accepted with changed data");
    changed_source.data = serde_json::json!({ "items": [{ "id": "b" }] });
    store
        .save_call_record(&WorkflowV2CallRecord::new(
            store.run_id(),
            source_record.call,
            source_record.attempt,
            source_record.input_hash,
            changed_source,
            source_record.depends_on,
        ))
        .expect("save changed source record");

    let second = runtime
        .run_serial_async_with_prepared_inputs(
            &executions,
            {
                let store = store.clone();
                move |execution| {
                    let store = store.clone();
                    async move { Ok(prepared_source_execution(execution, &store)) }
                }
            },
            |execution| async move {
                let mut result = accepted_result(&format!("{} reran", execution.call.id));
                result.data = serde_json::json!({
                    "observed_source": execution.input.get("source_data").cloned()
                });
                Ok(result)
            },
        )
        .await
        .expect("second run");

    assert_eq!(second.reused, 1);
    assert_eq!(second.executed, 1);
    let downstream = store
        .load_call_record("downstream")
        .expect("load downstream")
        .expect("downstream record");
    assert_eq!(downstream.attempt, 2);
    assert_eq!(
        downstream.result.data["observed_source"],
        serde_json::json!([{ "id": "b" }])
    );
}

fn prepared_source_execution(
    mut execution: WorkflowV2CallExecution,
    store: &WorkflowV2ResultStore,
) -> WorkflowV2CallExecution {
    if execution.call.id == "downstream" {
        let source = store
            .load_call_record("source")
            .expect("load source")
            .expect("source record");
        let source_data = source.result.data["items"].clone();
        execution.input = serde_json::json!({
            "call": "downstream",
            "source": "source.items",
            "source_data": source_data,
        });
    }
    execution
}
