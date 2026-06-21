use std::path::Path;

use archon_workflow::{
    LifecycleAction, LifecycleController, StageRunOutput, StageRunRequest, StageStatus,
    WorkflowExecutor, WorkflowPolicy, WorkflowSpec, WorkflowStageRunner, WorkflowStore,
    WriteBoundaryProbe,
};

fn git(args: &[&str], cwd: &Path) {
    let out = std::process::Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn canonical_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    git(&["init", "-q", "-b", "main"], dir.path());
    git(&["config", "user.name", "t"], dir.path());
    git(&["config", "user.email", "t@local"], dir.path());
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/seed.rs"), "// seed\n").unwrap();
    git(&["add", "-A"], dir.path());
    git(&["commit", "-q", "-m", "init"], dir.path());
    dir
}

struct CoverageRunner {
    implemented_ids: Vec<&'static str>,
}

impl WriteBoundaryProbe for CoverageRunner {
    fn supports_workspace_boundary(&self) -> bool {
        true
    }
}

struct MissingStatusRunner;

impl WriteBoundaryProbe for MissingStatusRunner {
    fn supports_workspace_boundary(&self) -> bool {
        true
    }
}

#[async_trait::async_trait]
impl WorkflowStageRunner for MissingStatusRunner {
    async fn run_stage(
        &self,
        request: StageRunRequest,
    ) -> archon_workflow::WorkflowResult<StageRunOutput> {
        let root = request.input["target_repository_root"].as_str().unwrap();
        let declared = request.input["write_coordination"]["declared_target_files"]
            .as_array()
            .unwrap();
        for file in declared {
            let path = Path::new(root).join(file.as_str().unwrap());
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "// implemented without status\n").unwrap();
        }
        let body = serde_json::json!({
            "implemented_task_ids": ["T040", "T050"],
            "changed_files": ["src/a.rs"],
            "commands_run": [{
                "role": "verification",
                "command": "generic verify src/a.rs",
                "exit_status": 0
            }],
            "residual_gaps": []
        });
        Ok(StageRunOutput::markdown(body.to_string()))
    }
}

#[async_trait::async_trait]
impl WorkflowStageRunner for CoverageRunner {
    async fn run_stage(
        &self,
        request: StageRunRequest,
    ) -> archon_workflow::WorkflowResult<StageRunOutput> {
        let root = request.input["target_repository_root"].as_str().unwrap();
        let declared = request.input["write_coordination"]["declared_target_files"]
            .as_array()
            .unwrap();
        for file in declared {
            let path = Path::new(root).join(file.as_str().unwrap());
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "// implemented\n").unwrap();
        }
        let body = serde_json::json!({
            "status": "accepted",
            "implemented_task_ids": self.implemented_ids,
            "changed_files": ["src/a.rs"],
            "commands_run": [{
                "role": "verification",
                "command": "generic verify src/a.rs",
                "exit_status": 0
            }],
            "residual_gaps": []
        });
        Ok(StageRunOutput::markdown(body.to_string()))
    }
}

struct SerialCoverageRunner {
    implemented_ids: Vec<&'static str>,
}

impl WriteBoundaryProbe for SerialCoverageRunner {}

#[async_trait::async_trait]
impl WorkflowStageRunner for SerialCoverageRunner {
    async fn run_stage(
        &self,
        request: StageRunRequest,
    ) -> archon_workflow::WorkflowResult<StageRunOutput> {
        let target = request.input["fanout_item"]["target_files"][0]
            .as_str()
            .unwrap();
        std::fs::create_dir_all(Path::new(target).parent().unwrap()).unwrap();
        std::fs::write(target, "// serial implemented\n").unwrap();
        let body = serde_json::json!({
            "status": "implemented",
            "implemented_task_ids": self.implemented_ids,
            "changed_files": [target],
            "commands_run": [{
                "role": "verification",
                "command": "generic verify serial coverage",
                "exit_status": 0
            }],
            "residual_gaps": []
        });
        Ok(StageRunOutput::markdown(body.to_string()))
    }
}

struct DirectCoverageRunner {
    target: String,
    implemented_ids: Vec<&'static str>,
}

impl WriteBoundaryProbe for DirectCoverageRunner {}

#[async_trait::async_trait]
impl WorkflowStageRunner for DirectCoverageRunner {
    async fn run_stage(
        &self,
        _request: StageRunRequest,
    ) -> archon_workflow::WorkflowResult<StageRunOutput> {
        std::fs::write(&self.target, "// direct implemented\n").unwrap();
        let body = serde_json::json!({
            "status": "implemented",
            "implemented_work_unit_ids": self.implemented_ids,
            "changed_files": [self.target],
            "commands_run": [{
                "role": "verification",
                "command": "generic verify direct coverage",
                "exit_status": 0
            }],
            "residual_gaps": []
        });
        Ok(StageRunOutput::markdown(body.to_string()))
    }
}

fn coverage_spec(canonical: &Path) -> WorkflowSpec {
    WorkflowSpec::from_yaml(&format!(
        r#"
schema: archon.workflow.v1
name: work-unit-coverage
task: Implement generic work units.
target_repository_root: "{}"
stages:
  - id: implement
    kind: fanout
    item_kind: implementation
    completion_task_ids: [T040, T050]
    input:
      items:
        - task_id: T040
          target_files:
            - src/a.rs
"#,
        canonical.display()
    ))
    .unwrap()
}

fn serial_coverage_spec(target: &Path) -> WorkflowSpec {
    WorkflowSpec::from_yaml(&format!(
        r#"
schema: archon.workflow.v1
name: serial-work-unit-coverage
task: Implement generic work units.
stages:
  - id: implement
    kind: fanout
    item_kind: implementation
    completion_task_ids: [T040, T050]
    input:
      items:
        - task_id: T040
          target_files:
            - "{}"
"#,
        target.display()
    ))
    .unwrap()
}

fn direct_coverage_spec(target: &Path) -> WorkflowSpec {
    WorkflowSpec::from_yaml(&format!(
        r#"
schema: archon.workflow.v1
name: direct-work-unit-coverage
task: Implement generic work units.
stages:
  - id: implement
    kind: implementation
    agent: workflow-coder
    required_work_units: [docs-linux-install, checkout-ui]
    expected_target_files:
      - "{}"
"#,
        target.display()
    ))
    .unwrap()
}

fn permissive_policy() -> WorkflowPolicy {
    WorkflowPolicy {
        require_human_for_dangerous_tools: false,
        ..WorkflowPolicy::default()
    }
}

#[tokio::test]
async fn patch_applied_without_required_work_unit_coverage_fails_stage() {
    let repo = canonical_repo();
    let store = WorkflowStore::project(repo.path());
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let run = executor.start(coverage_spec(repo.path())).unwrap();

    let report = executor
        .execute_with_runner(
            run.clone(),
            &CoverageRunner {
                implemented_ids: vec!["T040"],
            },
        )
        .await
        .unwrap();

    assert_eq!(report.failed, 1);
    let finished = store.load_state(&run.id).unwrap();
    let stage = finished.stages.get("implement").unwrap();
    assert_eq!(stage.status, StageStatus::Failed);
    assert!(
        stage.error.as_deref().unwrap_or_default().contains("T050"),
        "stage error should name missing required work unit: {:?}",
        stage.error
    );
    let coverage = stage
        .artifacts
        .iter()
        .find(|artifact| {
            artifact
                .path
                .to_string_lossy()
                .contains("work_unit_coverage")
        })
        .expect("coverage artifact should be attached");
    let body = std::fs::read_to_string(store.run_dir(&run.id).join(&coverage.path)).unwrap();
    assert!(body.contains(r#""missing_work_units": ["#), "{body}");
    assert!(body.contains("T050"), "{body}");
    let item_artifact = stage
        .artifacts
        .iter()
        .find(|artifact| artifact.path.to_string_lossy().contains("implement-0"))
        .expect("item artifact should be attached");
    let item_body = std::fs::read_to_string(store.run_dir(&run.id).join(&item_artifact.path))
        .expect("item artifact readable");
    assert!(item_body.contains("missing-work-unit:T050"), "{item_body}");
}

#[tokio::test]
async fn patch_applied_with_required_work_unit_coverage_accepts_stage() {
    let repo = canonical_repo();
    let store = WorkflowStore::project(repo.path());
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let run = executor.start(coverage_spec(repo.path())).unwrap();

    let report = executor
        .execute_with_runner(
            run.clone(),
            &CoverageRunner {
                implemented_ids: vec!["T040", "T050"],
            },
        )
        .await
        .unwrap();

    assert_eq!(report.failed, 0);
    let finished = store.load_state(&run.id).unwrap();
    let stage = finished.stages.get("implement").unwrap();
    assert_eq!(stage.status, StageStatus::Accepted);
}

#[tokio::test]
async fn implemented_ids_without_explicit_status_do_not_satisfy_coverage() {
    let repo = canonical_repo();
    let store = WorkflowStore::project(repo.path());
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let run = executor.start(coverage_spec(repo.path())).unwrap();

    let report = executor
        .execute_with_runner(run.clone(), &MissingStatusRunner)
        .await
        .unwrap();

    assert_eq!(report.failed, 1);
    let finished = store.load_state(&run.id).unwrap();
    let stage = finished.stages.get("implement").unwrap();
    assert_eq!(stage.status, StageStatus::Failed);
    assert!(
        stage.error.as_deref().unwrap_or_default().contains("T040"),
        "missing explicit status must leave units unsatisfied: {:?}",
        stage.error
    );
}

#[tokio::test]
async fn restart_reruns_applied_manifest_when_stage_coverage_is_incomplete() {
    let repo = canonical_repo();
    let store = WorkflowStore::project(repo.path());
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let run = executor.start(coverage_spec(repo.path())).unwrap();

    let first = executor
        .execute_with_runner(
            run.clone(),
            &CoverageRunner {
                implemented_ids: vec!["T040"],
            },
        )
        .await
        .unwrap();
    assert_eq!(first.failed, 1);
    let first_state = store.load_state(&run.id).unwrap();
    let first_artifact = first_state
        .stages
        .get("implement")
        .unwrap()
        .artifacts
        .iter()
        .find(|artifact| {
            artifact
                .path
                .to_string_lossy()
                .contains("work_unit_coverage")
        })
        .unwrap()
        .path
        .clone();

    let restarted = LifecycleController::new(store.clone())
        .apply(&run.id, LifecycleAction::RestartStage("implement".into()))
        .unwrap();
    assert!(
        !store.run_dir(&run.id).join(&first_artifact).exists(),
        "restart must archive stale coverage evidence out of active artifact path"
    );
    assert!(
        store.run_dir(&run.id).join("archived-attempts").exists(),
        "restart must leave an archived-attempts evidence trail"
    );
    let second = executor
        .execute_with_runner(
            restarted,
            &CoverageRunner {
                implemented_ids: vec!["T040", "T050"],
            },
        )
        .await
        .unwrap();

    assert_eq!(second.failed, 0);
    let finished = store.load_state(&run.id).unwrap();
    let stage = finished.stages.get("implement").unwrap();
    assert_eq!(stage.status, StageStatus::Accepted);
}

#[tokio::test]
async fn serial_fanout_partial_item_cannot_satisfy_stage_required_units() {
    let repo = canonical_repo();
    let target = repo.path().join("src/serial.rs");
    let store = WorkflowStore::project(repo.path());
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let run = executor.start(serial_coverage_spec(&target)).unwrap();

    let report = executor
        .execute_with_runner(
            run.clone(),
            &SerialCoverageRunner {
                implemented_ids: vec!["T040"],
            },
        )
        .await
        .unwrap();

    assert_eq!(report.failed, 1);
    let finished = store.load_state(&run.id).unwrap();
    let stage = finished.stages.get("implement").unwrap();
    assert_eq!(stage.status, StageStatus::Failed);
    assert!(
        stage.error.as_deref().unwrap_or_default().contains("T050"),
        "stage error should name missing aggregate unit: {:?}",
        stage.error
    );
}

#[tokio::test]
async fn direct_implementation_partial_work_unit_output_is_rejected() {
    let repo = canonical_repo();
    let target = repo.path().join("src/direct.rs");
    let store = WorkflowStore::project(repo.path());
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let run = executor.start(direct_coverage_spec(&target)).unwrap();

    let report = executor
        .execute_with_runner(
            run.clone(),
            &DirectCoverageRunner {
                target: target.display().to_string(),
                implemented_ids: vec!["docs-linux-install"],
            },
        )
        .await
        .unwrap();

    assert_eq!(report.failed, 1);
    let finished = store.load_state(&run.id).unwrap();
    let stage = finished.stages.get("implement").unwrap();
    assert_eq!(stage.status, StageStatus::Failed);
    assert!(
        stage
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("checkout-ui"),
        "stage error should name missing direct unit: {:?}",
        stage.error
    );
}
