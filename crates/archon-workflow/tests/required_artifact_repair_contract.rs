use archon_workflow::{
    RunStatus, StageRunOutput, StageRunRequest, StageStatus, WorkflowExecutor, WorkflowPolicy,
    WorkflowSpec, WorkflowStageRunner, WorkflowStore,
};

fn permissive_policy() -> WorkflowPolicy {
    WorkflowPolicy {
        require_human_for_dangerous_tools: false,
        ..WorkflowPolicy::default()
    }
}

fn target_files(request: &StageRunRequest) -> Vec<String> {
    request
        .input
        .get("fanout_item")
        .and_then(|item| item.get("target_files"))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(str::to_string)
        .collect()
}

fn spec_for(required: &str) -> WorkflowSpec {
    WorkflowSpec::from_yaml(&format!(
        r#"
schema: archon.workflow.v1
name: required-artifact-repair-contract
task: Verify required project artifacts.
stages:
  - id: final-quality
    kind: quality_gate
    required_artifacts:
      - "{required}"
"#,
    ))
    .unwrap()
}

#[tokio::test]
async fn blocked_required_artifact_repair_is_preserved_as_evidence() {
    struct BlockedRepairRunner;

    impl archon_workflow::WriteBoundaryProbe for BlockedRepairRunner {}
    #[async_trait::async_trait]
    impl WorkflowStageRunner for BlockedRepairRunner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            if request.stage_kind == archon_workflow::StageKind::Implementation {
                return Ok(StageRunOutput::markdown(
                    r#"{"status":"blocked","artifact_path":"dataset/manifest.json","reason":"missing validated provider data","commands_run":[{"command":"archon trading data fetch-native --provider openbb --symbol SPY --timeframe 1D --start 2024-01-01 --end 2024-02-01 --dataset-id openbb-SPY-1D-raw","exit_status":0,"output_summary":"unavailable; no dataset was created"}]}"#,
                ));
            }
            Ok(StageRunOutput::markdown("status: verified"))
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let target = temp
        .path()
        .join(".archon/trading-lab/data/datasets/*/*/manifest.json");
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let run = executor.start(spec_for(target.to_str().unwrap())).unwrap();
    let report = executor
        .execute_with_runner(run.clone(), &BlockedRepairRunner)
        .await
        .unwrap();

    assert_eq!(report.blocked, 1, "repair evidence should be blocked");
    assert_eq!(
        report.failed, 1,
        "final gate should still fail missing file"
    );
    let finished = store.load_state(&run.id).unwrap();
    assert_eq!(finished.status, RunStatus::Failed);
    assert_eq!(
        finished
            .stages
            .get("repair-required-artifacts")
            .unwrap()
            .status,
        StageStatus::Blocked
    );
    assert_eq!(
        finished.stages.get("final-quality").unwrap().status,
        StageStatus::Failed
    );
}

#[tokio::test]
async fn accepted_required_artifact_report_evidence_passes_repair_contract() {
    struct ReportRepairRunner;

    impl archon_workflow::WriteBoundaryProbe for ReportRepairRunner {}
    #[async_trait::async_trait]
    impl WorkflowStageRunner for ReportRepairRunner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            if request.stage_kind == archon_workflow::StageKind::Implementation {
                for target in target_files(&request) {
                    let path = std::path::Path::new(&target);
                    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                    std::fs::write(path, "report created with verified evidence").unwrap();
                }
                return Ok(StageRunOutput::markdown(
                    r#"{"status":"accepted","artifact":"acceptance-report.md","evidence":"created from required artifact inventory"}"#,
                ));
            }
            Ok(StageRunOutput::markdown("status: verified"))
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let target = temp
        .path()
        .join("tasks/demo/artifacts/acceptance-report.md");
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let run = executor.start(spec_for(target.to_str().unwrap())).unwrap();
    let report = executor
        .execute_with_runner(run.clone(), &ReportRepairRunner)
        .await
        .unwrap();

    assert_eq!(report.failed, 0);
    let finished = store.load_state(&run.id).unwrap();
    assert_eq!(finished.status, RunStatus::Completed);
    assert_eq!(
        finished
            .stages
            .get("repair-required-artifacts")
            .unwrap()
            .status,
        StageStatus::Accepted
    );
}

#[tokio::test]
async fn idempotent_required_artifact_noop_passes_when_target_exists() {
    struct NoopRepairRunner;

    impl archon_workflow::WriteBoundaryProbe for NoopRepairRunner {}
    #[async_trait::async_trait]
    impl WorkflowStageRunner for NoopRepairRunner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            if request.stage_kind == archon_workflow::StageKind::Implementation {
                return Ok(StageRunOutput::markdown(
                    r#"{"status":"accepted","idempotent_noop":true,"evidence":"required artifact already exists and is non-placeholder"}"#,
                ));
            }
            Ok(StageRunOutput::markdown("status: verified"))
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let target = temp
        .path()
        .join("tasks/demo/artifacts/acceptance-report.md");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(
        &target,
        "# Existing Report\n\nVerified artifact evidence already exists.\n",
    )
    .unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let run = executor.start(spec_for(target.to_str().unwrap())).unwrap();
    let report = executor
        .execute_with_runner(run.clone(), &NoopRepairRunner)
        .await
        .unwrap();

    assert_eq!(report.failed, 0);
    let finished = store.load_state(&run.id).unwrap();
    assert_eq!(finished.status, RunStatus::Completed);
    assert_eq!(
        finished
            .stages
            .get("repair-required-artifacts")
            .unwrap()
            .status,
        StageStatus::Accepted
    );
}

#[tokio::test]
async fn idempotent_required_artifact_noop_fails_when_target_missing() {
    struct NoopRepairRunner;

    impl archon_workflow::WriteBoundaryProbe for NoopRepairRunner {}
    #[async_trait::async_trait]
    impl WorkflowStageRunner for NoopRepairRunner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            if request.stage_kind == archon_workflow::StageKind::Implementation {
                return Ok(StageRunOutput::markdown(
                    r#"{"status":"accepted","idempotent_noop":true,"evidence":"required artifact already exists"}"#,
                ));
            }
            Ok(StageRunOutput::markdown("status: verified"))
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let target = temp
        .path()
        .join("tasks/demo/artifacts/acceptance-report.md");
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let run = executor.start(spec_for(target.to_str().unwrap())).unwrap();
    let report = executor
        .execute_with_runner(run.clone(), &NoopRepairRunner)
        .await
        .unwrap();

    assert_eq!(report.failed, 1);
    let finished = store.load_state(&run.id).unwrap();
    assert_eq!(finished.status, RunStatus::Failed);
    assert_eq!(
        finished
            .stages
            .get("repair-required-artifacts")
            .unwrap()
            .status,
        StageStatus::Failed
    );
}
