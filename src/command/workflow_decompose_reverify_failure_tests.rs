use super::*;
use archon_workflow::{WorkflowEventKind, WorkflowEventLog};

#[tokio::test]
async fn reverify_records_an_acceptance_error_after_started_event() {
    let fixture = super::fixture();
    let run_dir = fixture.store.run_dir(&fixture.run_id);
    let state_path = run_dir.join("state.json");
    let state_before = std::fs::read(&state_path).unwrap();
    let contract_path = fixture
        .project
        .join("tasks/PRD-X")
        .join(archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE);
    std::fs::write(&contract_path, "{}").unwrap();
    let env_vars = archon_core::env_vars::load_env_vars_from(&Default::default());
    let error = crate::command::workflow::workflow_decompose_reverify::reverify(
        &fixture.project,
        &fixture.run_id,
        &archon_core::config::ArchonConfig::default(),
        &env_vars,
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("reverify recorded operational_failure"),
        "{error}"
    );

    let latest = crate::command::workflow_decompose_latest_report::latest_report(
        &fixture.store,
        &fixture.run_id,
    )
    .unwrap()
    .unwrap();
    assert_eq!(latest.outcome, "operational_failure");
    let report_path = run_dir.join(format!("decomposition/reverify/reports/{}.json", latest.id));
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(report_path).unwrap()).unwrap();
    assert!(report["operational_failure_cause"].is_string());
    let record = WorkflowV2ResultStore::new(run_dir.join("v2"))
        .load_call_records()
        .unwrap()
        .into_iter()
        .find(|record| record.call.id == latest.id)
        .unwrap();
    assert_eq!(record.result.data["outcome"], "operational_failure");
    let events =
        archon_workflow::events::read_event_log(&fixture.store.events_path(&fixture.run_id))
            .unwrap()
            .0;
    assert!(events.iter().any(|event| {
        event.kind == WorkflowEventKind::DecompositionReverified
            && event.detail["report_id"] == latest.id
            && event.detail["outcome"] == "operational_failure"
            && event.detail["operational_failure_cause"].is_string()
    }));
    assert_eq!(std::fs::read(state_path).unwrap(), state_before);
    assert!(
        !run_dir
            .join(format!(
                "decomposition/reverify/remediation-{}.json",
                latest.id
            ))
            .exists()
    );
}

#[tokio::test]
async fn judge_error_is_published_redacted_and_next_reverify_supersedes_it() {
    const SECRET: &str = "tok-9f8e7d6c5b4a3210";
    let fixture = super::fixture();
    let run_dir = fixture.store.run_dir(&fixture.run_id);
    let state_path = run_dir.join("state.json");
    let mut paused_run = fixture.store.load_state(&fixture.run_id).unwrap();
    paused_run.status = RunStatus::Paused;
    std::fs::write(&state_path, serde_json::to_vec_pretty(&paused_run).unwrap()).unwrap();
    let state_before = std::fs::read(&state_path).unwrap();
    let run = fixture.store.load_state(&fixture.run_id).unwrap();
    let v2 = WorkflowV2ResultStore::new(run_dir.join("v2"));
    let prior: WorkflowV2CallRecord =
        serde_json::from_slice(&std::fs::read(&fixture.prior_record).unwrap()).unwrap();
    let lease =
        crate::command::workflow_executor_lease::acquire(&run_dir, &fixture.run_id).unwrap();
    let started_seq = fixture.store.next_event_seq(&fixture.run_id).unwrap();
    WorkflowEventLog::new(fixture.store.clone())
        .emit(
            &fixture.run_id,
            started_seq,
            WorkflowEventKind::DecompositionReverifyStarted,
            serde_json::json!({"invocation_id":"reverify-test"}),
        )
        .unwrap();
    let error = anyhow::anyhow!("FreezeIncomplete: provider authentication_error: token={SECRET}");
    let returned =
        crate::command::workflow::workflow_decompose_reverify::publish::record_operational_failure(
            &fixture.store,
            &fixture.run_id,
            &run_dir,
            &lease,
            run.generation,
            &run.status,
            &v2,
            &prior.call.id,
            &prior.output_hash,
            "reverify-test",
            &error,
        )
        .to_string();
    assert!(returned.contains("authentication_error"), "{returned}");
    let retry_command = format!("archon workflow reverify {}", fixture.run_id);
    assert!(returned.contains(&retry_command), "{returned}");
    assert_eq!(returned.matches(&retry_command).count(), 1, "{returned}");
    assert!(!returned.contains("Retry the freeze"), "{returned}");
    assert!(!returned.contains(SECRET), "{returned}");

    let latest = crate::command::workflow_decompose_latest_report::latest_report(
        &fixture.store,
        &fixture.run_id,
    )
    .unwrap()
    .unwrap();
    assert_eq!(latest.outcome, "operational_failure");
    let report_path = run_dir.join(format!("decomposition/reverify/reports/{}.json", latest.id));
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(report_path).unwrap()).unwrap();
    assert_eq!(
        report["operational_failure_cause"],
        "FreezeIncomplete: provider authentication_error: token=***REDACTED***"
    );
    assert!(
        report["operational_failure_cause"]
            .as_str()
            .unwrap()
            .contains("authentication_error")
    );
    assert!(!report.to_string().contains(SECRET));
    let record = v2
        .load_call_records()
        .unwrap()
        .into_iter()
        .find(|record| record.call.id == latest.id)
        .unwrap();
    assert_eq!(record.result.data["outcome"], "operational_failure");
    assert_eq!(
        record.result.data["operational_failure_cause"],
        report["operational_failure_cause"]
    );
    let events =
        archon_workflow::events::read_event_log(&fixture.store.events_path(&fixture.run_id))
            .unwrap()
            .0;
    let event = events
        .iter()
        .find(|event| event.kind == WorkflowEventKind::DecompositionReverified)
        .unwrap();
    assert_eq!(event.detail["outcome"], "operational_failure");
    assert_eq!(event.detail["report_id"], latest.id);
    assert!(
        event.detail["operational_failure_cause"]
            .as_str()
            .unwrap()
            .contains("authentication_error")
    );
    assert!(!event.detail.to_string().contains(SECRET));
    assert_eq!(std::fs::read(&state_path).unwrap(), state_before);
    assert_eq!(
        fixture.store.load_state(&fixture.run_id).unwrap().status,
        RunStatus::Paused
    );
    assert!(
        !run_dir
            .join(format!(
                "decomposition/reverify/remediation-{}.json",
                latest.id
            ))
            .exists()
    );

    drop(lease);
    let output = super::invoke_reverify(&super::archon_binary(), &fixture);
    let after = crate::command::workflow_decompose_latest_report::latest_report(
        &fixture.store,
        &fixture.run_id,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        after.superseded_id.as_deref(),
        Some(latest.id.as_str()),
        "reverify exit={} stdout={} stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_ne!(after.id, latest.id);
    assert!(output.status.success() || after.outcome == "operational_failure");
}

#[tokio::test]
async fn judge_error_is_preserved_when_failure_report_publication_fails() {
    let fixture = super::fixture();
    let run_dir = fixture.store.run_dir(&fixture.run_id);
    let run = fixture.store.load_state(&fixture.run_id).unwrap();
    let v2 = WorkflowV2ResultStore::new(run_dir.join("v2"));
    let prior: WorkflowV2CallRecord =
        serde_json::from_slice(&std::fs::read(&fixture.prior_record).unwrap()).unwrap();
    let lease =
        crate::command::workflow_executor_lease::acquire(&run_dir, &fixture.run_id).unwrap();

    let store = fixture.store.clone();
    let run_id = fixture.run_id.clone();
    crate::command::workflow::workflow_decompose_reverify::publish::set_before_final_publication_hook(
        move || {
            let mut changed = store.load_state(&run_id).unwrap();
            changed.generation += 1;
            store.save_state(&changed).unwrap();
        },
    );
    let error = anyhow::anyhow!("judge provider failed: original judge cause");
    let returned =
        crate::command::workflow::workflow_decompose_reverify::publish::record_operational_failure(
            &fixture.store,
            &fixture.run_id,
            &run_dir,
            &lease,
            run.generation,
            &run.status,
            &v2,
            &prior.call.id,
            &prior.output_hash,
            "reverify-publication-failure",
            &error,
        )
        .to_string();

    assert!(returned.contains("original judge cause"), "{returned}");
    assert!(returned.contains("ownership changed"), "{returned}");
    assert!(
        returned.contains("failure record was not written"),
        "{returned}"
    );
    assert!(
        !run_dir
            .join("decomposition/reverify/reports/reverify-publication-failure-report.json")
            .exists()
    );
}
