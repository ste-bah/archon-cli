use super::*;

#[path = "workflow_decompose_reverify_abandoned_tests.rs"]
mod abandoned_tests;

#[path = "workflow_decompose_reverify_eligibility_consumed_tests.rs"]
mod consumed_tests;

#[test]
fn reverify_error_for_running_run_names_status_and_next_action() {
    let fixture = fixture();
    let mut run = fixture.store.load_state(&fixture.run_id).unwrap();
    run.status = RunStatus::Running;
    fixture.store.save_state(&run).unwrap();

    let output = invoke_reverify(&archon_binary(), &fixture);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("status Running"), "{stderr}");
    assert!(stderr.contains("wait for it to finish"), "{stderr}");
}

#[tokio::test]
async fn unconsumed_reverify_ledger_allows_a_fresh_reverify() {
    let fixture = fixture();
    std::fs::write(&fixture.bodies[0], b"not a task body\n").unwrap();
    let binary = archon_binary();
    let first = invoke_reverify(&binary, &fixture);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let old_latest = crate::command::workflow_decompose_latest_report::latest_report(
        &fixture.store,
        &fixture.run_id,
    )
    .unwrap()
    .unwrap();
    assert_eq!(old_latest.outcome, "needs_remediation");
    assert_eq!(
        fixture.store.load_state(&fixture.run_id).unwrap().status,
        RunStatus::Paused
    );
    let old_ledger = fixture.store.run_dir(&fixture.run_id).join(format!(
        "decomposition/reverify/remediation-{}.json",
        old_latest.id
    ));
    assert!(old_ledger.is_file());

    let valid = format!(
        "```yaml\ntask_id: TASK-X-010\ntitle: T\ncomplexity: small\nstatus: ready\ndepends_on: []\nblocks: []\nimplements: [AC-X-001]\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n```\n\n# TASK-X-010\n\nA complete implementation.\n"
    );
    std::fs::write(&fixture.bodies[0], valid).unwrap();
    let output = invoke_reverify(&binary, &fixture);
    let new_latest = crate::command::workflow_decompose_latest_report::latest_report(
        &fixture.store,
        &fixture.run_id,
    )
    .unwrap()
    .unwrap();
    assert_ne!(new_latest.id, old_latest.id);
    let new_report_path = fixture.store.run_dir(&fixture.run_id).join(format!(
        "decomposition/reverify/reports/{}.json",
        new_latest.id
    ));
    assert!(new_report_path.is_file());
    assert!(old_ledger.is_file());
    let new_report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(new_report_path).unwrap()).unwrap();
    let status = fixture.store.load_state(&fixture.run_id).unwrap().status;
    match new_report["outcome"].as_str().unwrap() {
        "accepted" => {
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(status, RunStatus::Completed);
        }
        "needs_remediation" => {
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(status, RunStatus::Paused);
            assert!(
                fixture
                    .store
                    .run_dir(&fixture.run_id)
                    .join(format!(
                        "decomposition/reverify/remediation-{}.json",
                        new_latest.id
                    ))
                    .is_file()
            );
        }
        "operational_failure" => {
            assert!(!output.status.success());
            assert_eq!(status, RunStatus::Paused);
        }
        other => panic!("unexpected reverify outcome {other}"),
    }
    let state: FixedDecompositionStateV1 = serde_json::from_slice(
        &std::fs::read(
            fixture
                .store
                .run_dir(&fixture.run_id)
                .join("decomposition/state.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(state.phase, DecompositionPhase::Completed);
}

#[tokio::test]
async fn reverify_refuses_when_a_remediation_call_record_follows_the_report() {
    let fixture = fixture();
    std::fs::write(&fixture.bodies[0], b"not a task body\n").unwrap();
    let binary = archon_binary();
    let first = invoke_reverify(&binary, &fixture);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let store = WorkflowV2ResultStore::new(fixture.store.run_dir(&fixture.run_id).join("v2"));
    let mut result = WorkflowV2Result::default();
    result.status = WorkflowV2Status::Accepted;
    let mut record = WorkflowV2CallRecord::new(
        &fixture.run_id,
        WorkflowV2HostCall {
            id: "remediation-after-reverify".into(),
            method: WorkflowV2HostMethod::Implementation,
            write_mode: None,
            options: WorkflowV2HostOptions::default(),
        },
        1,
        "input".into(),
        result,
        Vec::new(),
    );
    record.started_at = "2000-01-01T00:00:00Z".into();
    record.finished_at = "2000-01-01T00:00:01Z".into();
    store.save_call_record(&record).unwrap();
    let output = invoke_reverify(&binary, &fixture);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("call-record snapshot"), "{stderr}");
    assert_eq!(
        fixture.store.load_state(&fixture.run_id).unwrap().status,
        RunStatus::Paused
    );
}

#[tokio::test]
async fn reverify_refuses_resume_trace_without_a_call_record() {
    let fixture = fixture();
    std::fs::write(&fixture.bodies[0], b"not a task body\n").unwrap();
    let binary = archon_binary();
    let first = invoke_reverify(&binary, &fixture);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let seq = fixture.store.next_event_seq(&fixture.run_id).unwrap();
    archon_workflow::WorkflowEventLog::new(fixture.store.clone())
        .emit(
            &fixture.run_id,
            seq,
            archon_workflow::WorkflowEventKind::Resumed,
            serde_json::json!({"resume_id": "crashed-before-first-call"}),
        )
        .unwrap();

    let output = invoke_reverify(&binary, &fixture);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("remediation already started"), "{stderr}");
    let records = WorkflowV2ResultStore::new(fixture.store.run_dir(&fixture.run_id).join("v2"))
        .load_call_records()
        .unwrap();
    assert!(
        !records
            .iter()
            .any(|record| record.call.id == "crashed-before-first-call")
    );
}

#[tokio::test]
async fn reverify_refuses_an_edited_report_even_when_v2_record_is_edited_too() {
    let fixture = fixture();
    let binary = archon_binary();
    let recorded = invoke_reverify(&binary, &fixture);
    assert!(!recorded.status.success());
    assert_eq!(
        crate::command::workflow_decompose_latest_report::latest_report(
            &fixture.store,
            &fixture.run_id,
        )
        .unwrap()
        .unwrap()
        .outcome,
        "operational_failure"
    );
    let report = crate::command::workflow_decompose_latest_report::latest_report(
        &fixture.store,
        &fixture.run_id,
    )
    .unwrap()
    .unwrap();
    let report_path = fixture
        .store
        .run_dir(&fixture.run_id)
        .join(format!("decomposition/reverify/reports/{}.json", report.id));
    let mut report_json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&report_path).unwrap()).unwrap();
    report_json["outcome"] = serde_json::json!("accepted");
    std::fs::write(
        &report_path,
        serde_json::to_vec_pretty(&report_json).unwrap(),
    )
    .unwrap();
    let v2 = WorkflowV2ResultStore::new(fixture.store.run_dir(&fixture.run_id).join("v2"));
    let record_path = v2.result_path(&report.id);
    let mut record_json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
    record_json["result"]["data"]["outcome"] = serde_json::json!("accepted");
    std::fs::write(
        &record_path,
        serde_json::to_vec_pretty(&record_json).unwrap(),
    )
    .unwrap();
    let mut run = fixture.store.load_state(&fixture.run_id).unwrap();
    run.status = RunStatus::Paused;
    fixture.store.save_state(&run).unwrap();

    let recovered = invoke_reverify(&binary, &fixture);
    assert!(!recovered.status.success());
    let stderr = String::from_utf8_lossy(&recovered.stderr);
    assert!(stderr.contains("event digest"), "{stderr}");
    assert_eq!(
        fixture.store.load_state(&fixture.run_id).unwrap().status,
        RunStatus::Paused
    );
}

#[tokio::test]
async fn paused_reverify_refuses_a_missing_remediation_ledger() {
    let fixture = fixture();
    std::fs::write(&fixture.bodies[0], b"not a task body\n").unwrap();
    let binary = archon_binary();
    let first = invoke_reverify(&binary, &fixture);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let latest = crate::command::workflow_decompose_latest_report::latest_report(
        &fixture.store,
        &fixture.run_id,
    )
    .unwrap()
    .unwrap();
    let ledger = fixture.store.run_dir(&fixture.run_id).join(format!(
        "decomposition/reverify/remediation-{}.json",
        latest.id
    ));
    std::fs::remove_file(&ledger).unwrap();
    let output = invoke_reverify(&binary, &fixture);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("ledger is missing"), "{stderr}");
}

#[tokio::test]
async fn paused_reverify_refuses_damaged_event_history_by_path() {
    let fixture = fixture();
    std::fs::write(&fixture.bodies[0], b"not a task body\n").unwrap();
    let binary = archon_binary();
    let first = invoke_reverify(&binary, &fixture);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let event_path = fixture.store.events_path(&fixture.run_id);
    use std::io::Write;
    std::fs::OpenOptions::new()
        .append(true)
        .open(&event_path)
        .unwrap()
        .write_all(b"{torn\n")
        .unwrap();
    let output = invoke_reverify(&binary, &fixture);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("damaged or torn"), "{stderr}");
    assert!(
        stderr.contains(&event_path.display().to_string()),
        "{stderr}"
    );
}

#[tokio::test]
async fn legacy_report_without_snapshot_is_eligible_when_reverified_event_is_last() {
    let fixture = fixture();
    std::fs::write(&fixture.bodies[0], b"not a task body\n").unwrap();
    let binary = archon_binary();
    let first = invoke_reverify(&binary, &fixture);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let latest = crate::command::workflow_decompose_latest_report::latest_report(
        &fixture.store,
        &fixture.run_id,
    )
    .unwrap()
    .unwrap();
    let run_dir = fixture.store.run_dir(&fixture.run_id);
    let report_path = run_dir.join(format!("decomposition/reverify/reports/{}.json", latest.id));
    let mut report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&report_path).unwrap()).unwrap();
    report.as_object_mut().unwrap().remove("report_event_seq");
    report
        .as_object_mut()
        .unwrap()
        .remove("call_record_ids_at_report");
    let bytes = serde_json::to_vec_pretty(&report).unwrap();
    std::fs::write(&report_path, &bytes).unwrap();
    let digest = archon_workflow::task_set_contract::content_digest(&bytes);
    let v2 = WorkflowV2ResultStore::new(run_dir.join("v2"));
    let record_path = v2.result_path(&latest.id);
    let mut record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
    record["input_hash"] = serde_json::json!(digest);
    record["result"]["data"] = report;
    std::fs::write(&record_path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
    let event_path = fixture.store.events_path(&fixture.run_id);
    let lines = std::fs::read_to_string(&event_path)
        .unwrap()
        .lines()
        .map(|line| {
            let mut event: serde_json::Value = serde_json::from_str(line).unwrap();
            if event["kind"] == "decomposition_reverified"
                && event["detail"]["report_id"] == latest.id
            {
                event["detail"]["report_digest"] = serde_json::json!(digest);
            }
            serde_json::to_string(&event).unwrap()
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    std::fs::write(&event_path, lines).unwrap();
    let latest = crate::command::workflow_decompose_latest_report::latest_report(
        &fixture.store,
        &fixture.run_id,
    )
    .unwrap();
    let v2 = WorkflowV2ResultStore::new(run_dir.join("v2"));
    let lease =
        crate::command::workflow_executor_lease::acquire(&run_dir, &fixture.run_id).unwrap();
    assert!(
        super::super::workflow_decompose_reverify::eligibility::paused_reverify_is_unconsumed(
            &fixture.store,
            &run_dir,
            &fixture.run_id,
            &lease,
            latest.as_ref(),
            &v2,
        )
        .unwrap()
    );
    archon_workflow::WorkflowEventLog::new(fixture.store.clone())
        .emit(
            &fixture.run_id,
            fixture.store.next_event_seq(&fixture.run_id).unwrap(),
            archon_workflow::WorkflowEventKind::Resumed,
            serde_json::json!({"resume_id": "later-event"}),
        )
        .unwrap();
    let refused =
        super::super::workflow_decompose_reverify::eligibility::paused_reverify_is_unconsumed(
            &fixture.store,
            &run_dir,
            &fixture.run_id,
            &lease,
            latest.as_ref(),
            &v2,
        );
    assert!(
        refused
            .unwrap_err()
            .to_string()
            .contains("remediation already started")
    );
}

#[tokio::test]
async fn operational_failure_report_allows_another_reverify() {
    let fixture = fixture();
    let binary = archon_binary();
    let first = invoke_reverify(&binary, &fixture);
    assert!(!first.status.success());
    let latest = crate::command::workflow_decompose_latest_report::latest_report(
        &fixture.store,
        &fixture.run_id,
    )
    .unwrap()
    .unwrap();
    assert_eq!(latest.outcome, "operational_failure");
    let mut run = fixture.store.load_state(&fixture.run_id).unwrap();
    run.status = RunStatus::Paused;
    fixture.store.save_state(&run).unwrap();
    let output = invoke_reverify(&binary, &fixture);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("is operational_failure"), "{stderr}");
    let next = crate::command::workflow_decompose_latest_report::latest_report(
        &fixture.store,
        &fixture.run_id,
    )
    .unwrap()
    .unwrap();
    assert_ne!(
        next.id, latest.id,
        "reverify was admitted and recorded a new report"
    );
    assert_eq!(next.outcome, "operational_failure");
    assert_eq!(
        fixture.store.load_state(&fixture.run_id).unwrap().status,
        RunStatus::Paused
    );
}

#[tokio::test]
async fn reverify_refuses_a_run_paused_for_another_reason() {
    let fixture = fixture();
    let mut run = fixture.store.load_state(&fixture.run_id).unwrap();
    run.status = RunStatus::Paused;
    fixture.store.save_state(&run).unwrap();
    let output = invoke_reverify(&archon_binary(), &fixture);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("is not completed"), "{stderr}");
    assert!(stderr.contains("ledger"), "{stderr}");
    assert!(stderr.contains("workflow resume"), "{stderr}");
}
