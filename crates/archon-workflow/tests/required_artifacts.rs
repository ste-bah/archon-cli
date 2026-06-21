use archon_workflow::{
    RunStatus, StageRunOutput, StageRunRequest, StageStatus, WorkflowExecutor, WorkflowPolicy,
    WorkflowSpec, WorkflowStageRunner, WorkflowStore,
};

#[test]
fn quality_gate_fails_when_required_glob_has_no_match() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let spec = required_glob_artifact_spec();
    let run = executor.start(spec).unwrap();

    let report = executor.execute(run.clone()).unwrap();
    assert_eq!(report.failed, 1);

    let finished = store.load_state(&run.id).unwrap();
    let gate = finished.stages.get("final-quality").unwrap();
    let error = gate.error.as_deref().unwrap_or_default();
    assert!(error.contains("quality gate missing required artifact"));
    assert!(error.contains("backtests/*/report.json"));
}

#[test]
fn quality_gate_accepts_required_glob_match() {
    let temp = tempfile::tempdir().unwrap();
    let report_path = temp
        .path()
        .join(".archon/trading-lab/strategies/AHDM-v1/backtests/run-1/report.json");
    std::fs::create_dir_all(report_path.parent().unwrap()).unwrap();
    std::fs::write(
        &report_path,
        r#"{"artifact":"backtest-report","status":"ready"}"#,
    )
    .unwrap();

    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let spec = required_glob_artifact_spec();
    let run = executor.start(spec).unwrap();

    let report = executor.execute(run.clone()).unwrap();
    assert_eq!(report.failed, 0);
}

#[test]
fn quality_gate_accepts_existing_json_without_domain_validation() {
    let temp = tempfile::tempdir().unwrap();
    let artifact = temp
        .path()
        .join(".archon/trading-lab/strategies/AHDM-v1/strategy-spec.json");
    std::fs::create_dir_all(artifact.parent().unwrap()).unwrap();
    std::fs::write(
        &artifact,
        r#"{
          "strategy_id": "AHDM-v1",
          "required_datasets": { "dataset_refs": [] }
        }"#,
    )
    .unwrap();

    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let spec = ahdm_strategy_artifact_spec();
    let run = executor.start(spec).unwrap();

    let report = executor.execute(run.clone()).unwrap();
    assert_eq!(report.failed, 0);

    let finished = store.load_state(&run.id).unwrap();
    let gate = finished.stages.get("final-quality").unwrap();
    assert_eq!(finished.status, RunStatus::Completed);
    assert_eq!(gate.status, StageStatus::Accepted);
}

#[test]
fn inventory_records_project_root_resolution_for_project_artifacts() {
    let project = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    std::fs::write(repo.path().join(".git"), "gitdir: elsewhere").unwrap();
    let artifact = project
        .path()
        .join(".archon/trading-lab/strategies/AHDM-v1/strategy-spec.json");
    std::fs::create_dir_all(artifact.parent().unwrap()).unwrap();
    std::fs::write(
        &artifact,
        r#"{"artifact":"strategy-spec","status":"ready"}"#,
    )
    .unwrap();

    let store = WorkflowStore::project(project.path());
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let spec = required_artifact_spec_with_repo(repo.path());
    let run = executor.start(spec).unwrap();

    let report = executor.execute(run.clone()).unwrap();
    assert_eq!(report.failed, 0);

    let body = std::fs::read_to_string(
        store
            .run_dir(&run.id)
            .join("artifacts/required-artifact-inventory/required-artifact-inventory.json"),
    )
    .unwrap();
    let value: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        value["project_root"].as_str(),
        Some(project.path().to_str().unwrap())
    );
    assert_eq!(
        value["repository_root"].as_str(),
        Some(repo.path().to_str().unwrap())
    );
    assert_eq!(value["missing"].as_array().unwrap().len(), 0);
    let checked = value["checked"].as_array().unwrap();
    assert_eq!(checked[0]["exists"].as_bool(), Some(true));
    assert_eq!(
        checked[0]["resolved"].as_str(),
        Some(artifact.to_str().unwrap())
    );
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
                    std::fs::write(
                        &target,
                        r#"{"artifact":"strategy-spec","status":"repaired"}"#,
                    )
                    .map_err(|err| archon_workflow::WorkflowError::StageFailed(err.to_string()))?;
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

#[tokio::test]
async fn project_artifact_repair_bypasses_repo_coordinator() {
    struct BoundaryCapableRepairRunner {
        expected_root: String,
    }

    impl archon_workflow::WriteBoundaryProbe for BoundaryCapableRepairRunner {
        fn supports_workspace_boundary(&self) -> bool {
            true
        }
    }
    #[async_trait::async_trait]
    impl WorkflowStageRunner for BoundaryCapableRepairRunner {
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
                assert!(
                    request
                        .input
                        .get("write_coordination")
                        .is_none_or(|value| value["enabled"].as_bool() != Some(true)),
                    "project artifact repair must run serially, not in repo isolation"
                );
                for target in target_files(&request) {
                    std::fs::create_dir_all(std::path::Path::new(&target).parent().unwrap())
                        .unwrap();
                    std::fs::write(
                        target,
                        r#"{"artifact":"strategy-spec","status":"repaired"}"#,
                    )
                    .unwrap();
                }
            }
            Ok(StageRunOutput::markdown("status: verified"))
        }
    }

    let project = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    std::fs::write(repo.path().join(".git"), "gitdir: elsewhere").unwrap();
    let store = WorkflowStore::project(project.path());
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let spec = required_artifact_spec_with_repo(repo.path());
    let run = executor.start(spec).unwrap();

    let report = executor
        .execute_with_runner(
            run.clone(),
            &BoundaryCapableRepairRunner {
                expected_root: project.path().display().to_string(),
            },
        )
        .await
        .unwrap();

    assert_eq!(report.failed, 0);
    assert!(
        project
            .path()
            .join(".archon/trading-lab/strategies/AHDM-v1/strategy-spec.json")
            .is_file()
    );
}

#[tokio::test]
async fn blocked_artifact_repair_evidence_reaches_final_gate() {
    struct BlockedArtifactRunner;

    impl archon_workflow::WriteBoundaryProbe for BlockedArtifactRunner {}
    #[async_trait::async_trait]
    impl WorkflowStageRunner for BlockedArtifactRunner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            if request.stage_id.starts_with("repair-required-artifacts-") {
                let target = target_files(&request)
                    .into_iter()
                    .next()
                    .unwrap_or_default();
                return Ok(StageRunOutput::markdown(format!(
                    r#"{{
                      "status":"blocked",
                      "artifact_path":"{target}",
                      "missing_evidence":["real provider-native dataset evidence is absent"],
                      "reason":"creating this artifact would be placeholder evidence",
                      "commands_run":[{{
                        "command":"archon trading data fetch-native --provider openbb --symbol SPY --timeframe 1D --start 2024-01-01 --end 2024-02-01 --dataset-id openbb-SPY-1D-raw",
                        "exit_status":0,
                        "output_summary":"command reported unavailable and did not create dataset artifacts"
                      }}]
                    }}"#
                )));
            }
            match request.stage_id.as_str() {
                "post-artifact-repair-tests" => Ok(StageRunOutput::markdown(
                    r#"{
                      "status":"failed",
                      "verified":false,
                      "commands_run":[{"command":"artifact existence check","result":"failed"}],
                      "residual_gaps":["required artifact remains blocked by missing evidence"]
                    }"#,
                )),
                "post-artifact-repair-review" => Ok(StageRunOutput::markdown(
                    r#"{
                      "verdict":"reject",
                      "status":"failed",
                      "residual_gaps":["required artifact remains blocked by missing evidence"]
                    }"#,
                )),
                _ => Ok(StageRunOutput::markdown("status: completed")),
            }
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let spec = required_artifact_spec();
    let run = executor.start(spec).unwrap();

    let report = executor
        .execute_with_runner(run.clone(), &BlockedArtifactRunner)
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
        StageStatus::Blocked
    );
    assert_eq!(
        finished
            .stages
            .get("post-artifact-repair-tests")
            .unwrap()
            .status,
        StageStatus::ForcedAccepted
    );
    assert_eq!(
        finished
            .stages
            .get("post-artifact-repair-review")
            .unwrap()
            .status,
        StageStatus::ForcedAccepted
    );
    assert_eq!(
        finished
            .stages
            .get("post-artifact-repair-report")
            .unwrap()
            .status,
        StageStatus::Accepted
    );
    assert_eq!(
        finished.stages.get("final-quality").unwrap().status,
        StageStatus::Failed
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
    enable_required_artifact_self_heal: true
    required_artifacts:
      - .archon/trading-lab/strategies/AHDM-v1/strategy-spec.json
"#,
    )
    .unwrap()
}

fn required_glob_artifact_spec() -> WorkflowSpec {
    WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: required-glob-artifact-gate
task: Verify required project artifact.
stages:
  - id: final-quality
    kind: quality_gate
    required_artifacts:
      - .archon/trading-lab/strategies/AHDM-v1/backtests/*/report.json
"#,
    )
    .unwrap()
}

fn ahdm_strategy_artifact_spec() -> WorkflowSpec {
    WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: ahdm-strategy-artifact-gate
task: Implement AHDM-v1 trading data lake.
stages:
  - id: final-quality
    kind: quality_gate
    required_artifacts:
      - .archon/trading-lab/strategies/AHDM-v1/strategy-spec.json
"#,
    )
    .unwrap()
}

fn required_artifact_spec_with_repo(repo: &std::path::Path) -> WorkflowSpec {
    WorkflowSpec::from_yaml(&format!(
        r#"
schema: archon.workflow.v1
name: required-artifact-gate-with-repo
task: Verify required project artifact.
target_repository_root: "{repo}"
stages:
  - id: final-quality
    kind: quality_gate
    enable_required_artifact_self_heal: true
    required_artifacts:
      - .archon/trading-lab/strategies/AHDM-v1/strategy-spec.json
"#,
        repo = repo.display()
    ))
    .unwrap()
}
