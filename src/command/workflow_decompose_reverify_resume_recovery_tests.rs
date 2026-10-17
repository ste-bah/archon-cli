use super::*;

#[path = "workflow_decompose_reverify_recovery_race_tests.rs"]
mod race_tests;

#[tokio::test]
async fn workflow_decompose_resume_recovers_accepted_report_without_execution() {
    let fixture = resume_fixture().await;
    let report_id = persist_accepted_reverify(&fixture);
    let factory = ResumeFactory::default();
    let recovery = invoke_resume_entry(&fixture, &factory).await.unwrap();
    assert!(recovery.contains(&report_id), "{recovery}");
    assert!(recovery.contains("execution was not started"), "{recovery}");
    assert_eq!(
        fixture.store.load_state(&fixture.run_id).unwrap().status,
        RunStatus::Completed
    );
    assert_eq!(factory.builds.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[tokio::test]
async fn resume_entry_refuses_missing_and_edited_accepted_reports() {
    for edit_report in [false, true] {
        let fixture = resume_fixture().await;
        let report_id = persist_accepted_reverify(&fixture);
        let report_path = fixture
            .store
            .run_dir(&fixture.run_id)
            .join(format!("decomposition/reverify/reports/{report_id}.json"));
        if edit_report {
            let mut report: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&report_path).unwrap()).unwrap();
            report["outcome"] = serde_json::json!("needs_remediation");
            std::fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        } else {
            std::fs::remove_file(&report_path).unwrap();
        }
        let state_path = fixture.store.state_path(&fixture.run_id);
        let before = std::fs::read(&state_path).unwrap();
        let factory = ResumeFactory::default();
        let error = invoke_resume_entry(&fixture, &factory).await.unwrap_err();
        assert!(
            error.to_string().contains(if edit_report {
                "event digest"
            } else {
                "is missing"
            }),
            "{error:#}"
        );
        assert_eq!(std::fs::read(&state_path).unwrap(), before);
        assert_eq!(factory.builds.load(std::sync::atomic::Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn resume_recovery_ignores_later_activity() {
    let fixture = resume_fixture().await;
    persist_accepted_reverify(&fixture);
    let seq = fixture.store.next_event_seq(&fixture.run_id).unwrap();
    archon_workflow::WorkflowEventLog::new(fixture.store.clone())
        .emit(
            &fixture.run_id,
            seq,
            archon_workflow::WorkflowEventKind::Resumed,
            serde_json::json!({"resume_id": "later-activity"}),
        )
        .unwrap();
    let state_path = fixture.store.state_path(&fixture.run_id);
    let before = std::fs::read(&state_path).unwrap();
    let factory = ResumeFactory::default();
    let result = invoke_resume_entry(&fixture, &factory).await;
    assert!(
        !result
            .as_ref()
            .is_ok_and(|message| message.contains("Recovered accepted"))
    );
    assert!(
        result.is_err(),
        "resume unexpectedly completed without recovery"
    );
    assert_eq!(std::fs::read(&state_path).unwrap(), before);
}

#[tokio::test]
async fn resume_does_not_recover_accepted_report_when_call_snapshot_changed() {
    let fixture = resume_fixture().await;
    let report_id = persist_accepted_reverify(&fixture);
    let v2 = WorkflowV2ResultStore::new(fixture.store.run_dir(&fixture.run_id).join("v2"));
    let mut result = WorkflowV2Result::default();
    result.status = WorkflowV2Status::Accepted;
    result.data = serde_json::json!({"later": true});
    v2.save_call_record(&WorkflowV2CallRecord::new(
        &fixture.run_id,
        WorkflowV2HostCall {
            id: "later-call".into(),
            method: WorkflowV2HostMethod::FinalReport,
            write_mode: None,
            options: WorkflowV2HostOptions::default(),
        },
        2,
        "later-call-hash".into(),
        result,
        Vec::new(),
    ))
    .unwrap();

    // The call record arrived without a later workflow event, so event-last
    // alone is insufficient to recover the accepted report.
    let factory = ResumeFactory::default();
    let outcome = invoke_resume_entry(&fixture, &factory).await;
    assert!(
        !outcome
            .as_ref()
            .is_ok_and(|message| message.contains("Recovered accepted")),
        "resume unexpectedly recovered {report_id}: {outcome:?}"
    );
    assert_ne!(
        fixture.store.load_state(&fixture.run_id).unwrap().status,
        RunStatus::Completed
    );
}

#[tokio::test]
async fn resume_recovery_ignores_needs_remediation_report() {
    let fixture = resume_fixture().await;
    let report_id = persist_needs_remediation_reverify(&fixture);
    let latest = crate::command::workflow_decompose_latest_report::latest_report(
        &fixture.store,
        &fixture.run_id,
    )
    .unwrap()
    .unwrap();
    assert_eq!(latest.id, report_id);
    assert_eq!(latest.outcome, "needs_remediation");
    let factory = ResumeFactory::default();
    let result = invoke_resume_entry(&fixture, &factory).await;
    assert!(
        !result
            .as_ref()
            .is_ok_and(|message| message.contains("Recovered accepted"))
    );
    assert!(
        result.is_err(),
        "resume unexpectedly completed without recovery"
    );
    let error = format!("{:#}", result.unwrap_err());
    assert!(
        error.contains("resume recovery test provider barrier"),
        "resume failed before reaching the post-recovery executor barrier: {error}"
    );
    assert_eq!(
        factory.builds.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "resume must pass recovery decision and build its provider"
    );
}

#[derive(Default)]
struct ResumeFactory {
    builds: std::sync::atomic::AtomicUsize,
}

/// Use the production launch entry point so resume admission sees the same
/// bundle, launch arguments, catalog, route, metadata, and log files as a real
/// fixed-decomposition run. The provider barrier stops execution after those
/// launch records have been persisted.
async fn resume_fixture() -> Fixture {
    use std::path::Path;

    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().canonicalize().unwrap();
    let prd = project.join("prds/PRD-X.md");
    let task_root = project.join("tasks/PRD-X");
    std::fs::create_dir_all(prd.parent().unwrap()).unwrap();
    std::fs::create_dir_all(&task_root).unwrap();
    std::fs::write(&prd, super::PRD).unwrap();
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(&project)
            .status()
            .unwrap()
            .success()
    );
    let acceptance =
        crate::command::workflow_decompose_frozen_chain::test_support::freeze_acceptance(
            &project, &prd, &task_root,
        );
    crate::command::workflow_decompose_frozen_chain::test_support::freeze_skeleton(
        &project,
        &task_root,
        &acceptance,
        &["TASK-X-010", "TASK-X-020"],
    );
    let bodies = ["TASK-X-010.md", "TASK-X-020.md"]
        .into_iter()
        .map(|name| {
            let path = task_root.join(name);
            let task_id = name.trim_end_matches(".md");
            let body = format!(
                "```yaml\ntask_id: {task_id}\ntitle: T\ncomplexity: small\nstatus: ready\ndepends_on: []\nblocks: []\nimplements: [AC-X-001]\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n```\n\n# {task_id}\n\nA complete implementation.\n"
            );
            std::fs::write(&path, body).unwrap();
            path
        })
        .collect::<Vec<_>>();
    // The launch helper writes the run's complete persisted snapshot before
    // asking this factory to build a provider client.
    let mut config = archon_core::config::ArchonConfig::default();
    config.workflow.repository_root = Some(project.clone());
    let launch_factory = ResumeFactory::default();
    let _ = crate::command::workflow_decompose::run_fixed_decomposition_with_factory(
        &project,
        Path::new("prds/PRD-X.md"),
        Path::new("tasks/PRD-X"),
        None,
        true,
        &config,
        &archon_core::env_vars::load_env_vars_from(&std::collections::HashMap::new()),
        &launch_factory,
    )
    .await;
    let store = archon_workflow::WorkflowStore::project(&project);
    let mut run = store.list_runs().unwrap().pop().expect("launched run");
    let run_id = run.id.clone();
    run.status = RunStatus::Completed;
    run.updated_at = chrono::Utc::now();
    store.save_state(&run).unwrap();
    let state_path = store
        .run_dir(&run_id)
        .join(crate::command::workflow_decompose::FIXED_DECOMPOSITION_STATE_PATH);
    let mut state: FixedDecompositionStateV1 =
        serde_json::from_slice(&std::fs::read(&state_path).unwrap()).unwrap();
    state.phase = DecompositionPhase::Completed;
    store
        .write_run_json(
            &run_id,
            crate::command::workflow_decompose::FIXED_DECOMPOSITION_STATE_PATH,
            &state,
        )
        .unwrap();

    let v2 = WorkflowV2ResultStore::new(store.run_dir(&run_id).join("v2"));
    let mut result = WorkflowV2Result::default();
    result.status = WorkflowV2Status::Accepted;
    result.summary = "prior accepted report".into();
    result.data = serde_json::json!({"outcome":"accepted"});
    v2.save_call_record(&WorkflowV2CallRecord::new(
        &run_id,
        WorkflowV2HostCall {
            id: "prior-final-report".into(),
            method: WorkflowV2HostMethod::FinalReport,
            write_mode: None,
            options: WorkflowV2HostOptions::default(),
        },
        1,
        "prior-input".into(),
        result,
        Vec::new(),
    ))
    .unwrap();
    let prior_record = v2.result_path("prior-final-report");
    Fixture {
        _temp: temp,
        project,
        store,
        run_id,
        bodies,
        prior_record,
    }
}

#[async_trait::async_trait(?Send)]
impl archon_workflow::WorkflowLlmClientFactory for ResumeFactory {
    async fn build_client(
        &self,
        _request: archon_workflow::WorkflowLlmClientRequest,
    ) -> archon_workflow::WorkflowResult<std::sync::Arc<dyn archon_workflow::WorkflowLlmClient>>
    {
        use std::sync::atomic::Ordering;
        self.builds.fetch_add(1, Ordering::SeqCst);
        Err(archon_workflow::WorkflowError::port(
            "resume recovery test provider barrier",
        ))
    }
}

async fn invoke_resume_entry(
    fixture: &Fixture,
    factory: &dyn archon_workflow::WorkflowLlmClientFactory,
) -> anyhow::Result<String> {
    let fixed_state: FixedDecompositionStateV1 = serde_json::from_slice(&std::fs::read(
        fixture
            .store
            .run_dir(&fixture.run_id)
            .join(crate::command::workflow_decompose::FIXED_DECOMPOSITION_STATE_PATH),
    )?)?;
    crate::command::workflow_decompose::resume_fixed_decomposition_at_binary_revision(
        &fixture.project,
        &fixture.run_id,
        true,
        &archon_core::config::ArchonConfig::default(),
        &archon_core::env_vars::load_env_vars_from(&std::collections::HashMap::new()),
        factory,
        crate::command::workflow_decompose_progress::DecompositionCliUiSink::shared(),
        None,
        None,
        &fixed_state.identity.starting_binary_revision,
    )
    .await
}

fn persist_accepted_reverify(fixture: &Fixture) -> String {
    let report_id = "reverify-accepted-crash";
    let seq = fixture.store.next_event_seq(&fixture.run_id).unwrap();
    let v2 = WorkflowV2ResultStore::new(fixture.store.run_dir(&fixture.run_id).join("v2"));
    let mut call_record_ids_at_report = v2
        .load_call_records()
        .unwrap()
        .into_iter()
        .map(|record| record.call.id)
        .collect::<Vec<_>>();
    call_record_ids_at_report.push(report_id.to_string());
    call_record_ids_at_report.sort();
    let report = serde_json::json!({
        "schema_version": 2,
        "id": report_id,
        "outcome": "accepted",
        "supersedes_id": "prior-final-report",
        "report_event_seq": seq,
        "call_record_ids_at_report": call_record_ids_at_report
    });
    let report_path = fixture
        .store
        .run_dir(&fixture.run_id)
        .join(format!("decomposition/reverify/reports/{report_id}.json"));
    std::fs::create_dir_all(report_path.parent().unwrap()).unwrap();
    let bytes = serde_json::to_vec_pretty(&report).unwrap();
    std::fs::write(&report_path, &bytes).unwrap();
    let digest = archon_workflow::task_set_contract::content_digest(&bytes);
    let mut result = WorkflowV2Result::default();
    result.status = WorkflowV2Status::Accepted;
    result.data = report;
    v2.save_call_record(&WorkflowV2CallRecord::new(
        &fixture.run_id,
        WorkflowV2HostCall {
            id: report_id.into(),
            method: WorkflowV2HostMethod::FinalReport,
            write_mode: None,
            options: WorkflowV2HostOptions::default(),
        },
        2,
        digest.clone(),
        result,
        Vec::new(),
    ))
    .unwrap();
    archon_workflow::WorkflowEventLog::new(fixture.store.clone())
        .emit(
            &fixture.run_id,
            seq,
            archon_workflow::WorkflowEventKind::DecompositionReverified,
            serde_json::json!({
                "report_id": report_id,
                "supersedes_id": "prior-final-report",
                "report_digest": digest
            }),
        )
        .unwrap();
    let mut run = fixture.store.load_state(&fixture.run_id).unwrap();
    run.status = RunStatus::Paused;
    fixture.store.save_state(&run).unwrap();

    report_id.to_string()
}

fn persist_needs_remediation_reverify(fixture: &Fixture) -> String {
    let report_id = "reverify-needs-remediation-crash";
    let seq = fixture.store.next_event_seq(&fixture.run_id).unwrap();
    let v2 = WorkflowV2ResultStore::new(fixture.store.run_dir(&fixture.run_id).join("v2"));
    let mut call_record_ids_at_report = v2
        .load_call_records()
        .unwrap()
        .into_iter()
        .map(|record| record.call.id)
        .collect::<Vec<_>>();
    call_record_ids_at_report.push(report_id.to_string());
    call_record_ids_at_report.sort();
    let report = serde_json::json!({
        "schema_version": 2,
        "id": report_id,
        "outcome": "needs_remediation",
        "supersedes_id": "prior-final-report",
        "report_event_seq": seq,
        "call_record_ids_at_report": call_record_ids_at_report
    });
    let report_path = fixture
        .store
        .run_dir(&fixture.run_id)
        .join(format!("decomposition/reverify/reports/{report_id}.json"));
    std::fs::create_dir_all(report_path.parent().unwrap()).unwrap();
    let bytes = serde_json::to_vec_pretty(&report).unwrap();
    std::fs::write(&report_path, &bytes).unwrap();
    let digest = archon_workflow::task_set_contract::content_digest(&bytes);
    let mut result = WorkflowV2Result::default();
    result.status = WorkflowV2Status::NeedsReview;
    result.data = report;
    v2.save_call_record(&WorkflowV2CallRecord::new(
        &fixture.run_id,
        WorkflowV2HostCall {
            id: report_id.into(),
            method: WorkflowV2HostMethod::FinalReport,
            write_mode: None,
            options: WorkflowV2HostOptions::default(),
        },
        2,
        digest.clone(),
        result,
        Vec::new(),
    ))
    .unwrap();
    archon_workflow::WorkflowEventLog::new(fixture.store.clone())
        .emit(
            &fixture.run_id,
            seq,
            archon_workflow::WorkflowEventKind::DecompositionReverified,
            serde_json::json!({
                "report_id": report_id,
                "supersedes_id": "prior-final-report",
                "report_digest": digest
            }),
        )
        .unwrap();
    let ledger_path = fixture.store.run_dir(&fixture.run_id).join(format!(
        "decomposition/reverify/remediation-{report_id}.json"
    ));
    std::fs::write(
        ledger_path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "report_id": report_id,
            "subjects": ["TASK-X-010"]
        }))
        .unwrap(),
    )
    .unwrap();
    let mut run = fixture.store.load_state(&fixture.run_id).unwrap();
    run.status = RunStatus::Paused;
    fixture.store.save_state(&run).unwrap();

    report_id.to_string()
}
