use archon_workflow::{
    RunStatus, StageRunOutput, StageRunRequest, StageStatus, WorkflowExecutor, WorkflowPolicy,
    WorkflowSpec, WorkflowStageRunner, WorkflowStore,
};

#[test]
fn quality_gate_fails_when_required_project_artifact_is_missing() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let spec = required_artifact_spec();
    let run = executor.start(spec).unwrap();

    let report = executor.execute(run.clone()).unwrap();
    assert_eq!(report.failed, 1);

    let finished = store.load_state(&run.id).unwrap();
    let gate = finished.stages.get("final-quality").unwrap();
    assert_eq!(finished.status, RunStatus::Failed);
    assert_eq!(gate.status, StageStatus::Failed);
    let error = gate.error.as_deref().unwrap_or_default();
    assert!(error.contains("quality gate missing required artifact"));
    assert!(error.contains(".archon/trading-lab/strategies/AHDM-v1/strategy-spec.json"));

    let quality = std::fs::read_to_string(
        store
            .run_dir(&run.id)
            .join("quality")
            .join("final-quality.json"),
    )
    .unwrap();
    assert!(quality.contains("\"status\": \"failed\""));
    assert!(quality.contains("strategy-spec.json"));
}

#[test]
fn quality_gate_accepts_existing_required_project_artifact() {
    let temp = tempfile::tempdir().unwrap();
    let artifact = temp
        .path()
        .join(".archon/trading-lab/strategies/AHDM-v1/strategy-spec.json");
    std::fs::create_dir_all(artifact.parent().unwrap()).unwrap();
    std::fs::write(&artifact, "{}").unwrap();

    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let spec = required_artifact_spec();
    let run = executor.start(spec).unwrap();

    let report = executor.execute(run.clone()).unwrap();
    assert_eq!(report.failed, 0);

    let finished = store.load_state(&run.id).unwrap();
    let gate = finished.stages.get("final-quality").unwrap();
    assert_eq!(finished.status, RunStatus::Completed);
    assert_eq!(gate.status, StageStatus::Accepted);
}

#[tokio::test]
async fn missing_required_artifact_self_heals_before_final_gate() {
    struct ArtifactRepairRunner {
        expected_root: String,
    }

    impl archon_workflow::WriteBoundaryProbe for ArtifactRepairRunner {}
    #[async_trait::async_trait]
    impl WorkflowStageRunner for ArtifactRepairRunner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            if request.stage_kind == archon_workflow::StageKind::Implementation {
                assert_eq!(
                    request
                        .input
                        .get("target_repository_root")
                        .and_then(serde_json::Value::as_str),
                    Some(self.expected_root.as_str())
                );
                for target in target_files(&request) {
                    if let Some(parent) = std::path::Path::new(&target).parent() {
                        std::fs::create_dir_all(parent).map_err(|err| {
                            archon_workflow::WorkflowError::StageFailed(err.to_string())
                        })?;
                    }
                    std::fs::write(&target, "built by artifact repair").map_err(|err| {
                        archon_workflow::WorkflowError::StageFailed(err.to_string())
                    })?;
                }
                return Ok(StageRunOutput::markdown("status: completed"));
            }
            Ok(StageRunOutput::markdown("status: verified"))
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let artifact = temp
        .path()
        .join(".archon/trading-lab/strategies/AHDM-v1/strategy-spec.json");
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let spec = required_artifact_spec();
    let run = executor.start(spec).unwrap();

    let report = executor
        .execute_with_runner(
            run.clone(),
            &ArtifactRepairRunner {
                expected_root: temp.path().display().to_string(),
            },
        )
        .await
        .unwrap();
    assert_eq!(report.failed, 0);
    assert!(artifact.exists());

    let finished = store.load_state(&run.id).unwrap();
    assert_eq!(finished.status, RunStatus::Completed);
    assert_eq!(
        finished.stages.get("final-quality").unwrap().status,
        StageStatus::Accepted
    );
    assert_eq!(
        finished
            .stages
            .get("repair-required-artifacts")
            .unwrap()
            .status,
        StageStatus::Accepted
    );
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

fn permissive_policy() -> WorkflowPolicy {
    WorkflowPolicy {
        require_human_for_dangerous_tools: false,
        ..WorkflowPolicy::default()
    }
}

fn required_artifact_spec() -> WorkflowSpec {
    WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: required-artifact-gate
task: Verify required project artifact.
stages:
  - id: final-quality
    kind: quality_gate
    required_artifacts:
      - .archon/trading-lab/strategies/AHDM-v1/strategy-spec.json
"#,
    )
    .unwrap()
}
