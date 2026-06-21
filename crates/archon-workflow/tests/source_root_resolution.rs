use std::path::PathBuf;

use archon_workflow::{
    RunStatus, StageKind, StageRunOutput, StageRunRequest, StageStatus, WorkflowExecutor,
    WorkflowPolicy, WorkflowSpec, WorkflowStageRunner, WorkflowStore,
};

fn implementation_evidence(
    work_unit_id: &str,
    changed_files: &[&str],
    artifacts: &[&str],
) -> StageRunOutput {
    StageRunOutput::markdown(
        serde_json::json!({
            "status": "implemented",
            "implemented_work_unit_ids": [work_unit_id],
            "changed_files": changed_files,
            "artifacts": artifacts,
            "commands_run": [{
                "role": "verification",
                "command": format!("fixture verifies {work_unit_id}"),
                "exit_status": 0
            }],
            "residual_gaps": []
        })
        .to_string(),
    )
}

struct RootAssertingRunner {
    repo: PathBuf,
}

impl archon_workflow::WriteBoundaryProbe for RootAssertingRunner {}
#[async_trait::async_trait]
impl WorkflowStageRunner for RootAssertingRunner {
    async fn run_stage(
        &self,
        request: StageRunRequest,
    ) -> archon_workflow::WorkflowResult<StageRunOutput> {
        if request.stage_id == "focused_tests" {
            return Ok(StageRunOutput::markdown("focused tests passed"));
        }
        assert_eq!(request.stage_kind, StageKind::Implementation);
        let root = request.input["target_repository_root"].as_str().unwrap();
        assert_eq!(root, self.repo.display().to_string());
        let target = self.repo.join("src/lib.rs");
        std::fs::write(&target, "pub fn implemented() {}").unwrap();
        Ok(implementation_evidence(
            "root-resolution",
            &[target.to_str().unwrap()],
            &[],
        ))
    }
}

#[tokio::test]
async fn implementation_root_can_come_from_stage_text() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(project.join(".archon/workflows")).unwrap();
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(repo.join("Cargo.toml"), "[package]\nname = \"root-test\"\n").unwrap();

    let spec = WorkflowSpec::from_yaml(&format!(
        r#"
schema: archon.workflow.v1
name: recovered-root
task: recovery continuation without a repository path
stages:
  - id: implement
    kind: implementation
    task: Patch the recovered source file.
    required_work_units: [root-resolution]
    expected_target_files: ["src/lib.rs"]
  - id: focused_tests
    kind: agent
    task: "Run focused tests from {repo}"
    depends_on: [implement]
"#,
        repo = repo.display()
    ))
    .unwrap();

    let store = WorkflowStore::project(&project);
    let executor = WorkflowExecutor::new(
        store.clone(),
        WorkflowPolicy {
            require_human_for_dangerous_tools: false,
            ..WorkflowPolicy::default()
        },
    );
    let run = executor.start(spec).unwrap();
    let run_id = run.id.clone();
    executor
        .execute_with_runner(run, &RootAssertingRunner { repo: repo.clone() })
        .await
        .unwrap();

    let state = store.load_state(&run_id).unwrap();
    assert_eq!(
        state.stages.get("implement").unwrap().status,
        StageStatus::Accepted
    );
}

#[tokio::test]
async fn implementation_root_can_come_from_absolute_source_path() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let repo = temp.path().join("repo");
    let source = repo.join("src/lib.rs");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(source.parent().unwrap()).unwrap();
    std::fs::write(repo.join("Cargo.toml"), "[package]\nname = \"root-test\"\n").unwrap();

    let spec = WorkflowSpec::from_yaml(&format!(
        r#"
schema: archon.workflow.v1
name: recovered-root-from-file
task: recovery continuation without a repository path
stages:
  - id: implement
    kind: implementation
    task: "Patch {source}"
    required_work_units: [root-resolution]
    expected_target_files: ["src/lib.rs"]
"#,
        source = source.display()
    ))
    .unwrap();

    let store = WorkflowStore::project(&project);
    let executor = WorkflowExecutor::new(
        store.clone(),
        WorkflowPolicy {
            require_human_for_dangerous_tools: false,
            ..WorkflowPolicy::default()
        },
    );
    let run = executor.start(spec).unwrap();
    let run_id = run.id.clone();
    executor
        .execute_with_runner(run, &RootAssertingRunner { repo: repo.clone() })
        .await
        .unwrap();

    let state = store.load_state(&run_id).unwrap();
    assert_eq!(
        state.stages.get("implement").unwrap().status,
        StageStatus::Accepted
    );
}

#[tokio::test]
async fn implementation_root_prefers_explicit_top_level_root() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(repo.join("Cargo.toml"), "[package]\nname = \"root-test\"\n").unwrap();

    let spec = WorkflowSpec::from_yaml(&format!(
        r#"
schema: archon.workflow.v1
name: explicit-root
task: recovery continuation without embedded paths
target_repository_root: "{repo}"
stages:
  - id: implement
    kind: implementation
    task: Patch the recovered source file.
    required_work_units: [root-resolution]
    expected_target_files: ["src/lib.rs"]
"#,
        repo = repo.display()
    ))
    .unwrap();

    let store = WorkflowStore::project(&project);
    let executor = WorkflowExecutor::new(
        store.clone(),
        WorkflowPolicy {
            require_human_for_dangerous_tools: false,
            ..WorkflowPolicy::default()
        },
    );
    let run = executor.start(spec).unwrap();
    let run_id = run.id.clone();
    executor
        .execute_with_runner(run, &RootAssertingRunner { repo: repo.clone() })
        .await
        .unwrap();

    let state = store.load_state(&run_id).unwrap();
    assert_eq!(
        state.stages.get("implement").unwrap().status,
        StageStatus::Accepted
    );
}

#[tokio::test]
async fn implementation_stage_project_artifact_overrides_top_level_repo_root() {
    struct ProjectArtifactRunner {
        project: PathBuf,
        artifact: PathBuf,
    }

    impl archon_workflow::WriteBoundaryProbe for ProjectArtifactRunner {}
    #[async_trait::async_trait]
    impl WorkflowStageRunner for ProjectArtifactRunner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            assert_eq!(request.stage_kind, StageKind::Implementation);
            let root = request.input["target_repository_root"].as_str().unwrap();
            assert_eq!(root, self.project.display().to_string());
            std::fs::create_dir_all(self.artifact.parent().unwrap()).unwrap();
            std::fs::write(&self.artifact, "{\"ok\":true}").unwrap();
            Ok(implementation_evidence(
                "project-artifact",
                &[],
                &[self.artifact.to_str().unwrap()],
            ))
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let repo = temp.path().join("repo");
    let artifact = project.join(".archon/trading-lab/strategies/AHDM-v1/strategy-spec.json");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(repo.join("Cargo.toml"), "[package]\nname = \"root-test\"\n").unwrap();

    let spec = WorkflowSpec::from_yaml(&format!(
        r#"
schema: archon.workflow.v1
name: project-artifact-root
task: Repair a project artifact while source work targets a repository.
target_repository_root: "{repo}"
stages:
  - id: repair_project_artifact
    kind: implementation
    task: Create the required project artifact.
    required_work_units: [project-artifact]
    expected_target_files: ["{artifact}"]
"#,
        artifact = artifact.display(),
        repo = repo.display(),
    ))
    .unwrap();

    let store = WorkflowStore::project(&project);
    let executor = WorkflowExecutor::new(
        store.clone(),
        WorkflowPolicy {
            require_human_for_dangerous_tools: false,
            ..WorkflowPolicy::default()
        },
    );
    let run = executor.start(spec).unwrap();
    let run_id = run.id.clone();
    executor
        .execute_with_runner(
            run,
            &ProjectArtifactRunner {
                project: project.clone(),
                artifact: artifact.clone(),
            },
        )
        .await
        .unwrap();

    let state = store.load_state(&run_id).unwrap();
    assert_eq!(
        state.stages.get("repair_project_artifact").unwrap().status,
        StageStatus::Accepted
    );
    assert!(artifact.exists());
}

#[tokio::test]
async fn fanout_stage_project_artifact_targets_use_stage_fallback_root() {
    struct FanoutProjectArtifactRunner {
        project: PathBuf,
        artifact: PathBuf,
    }

    impl archon_workflow::WriteBoundaryProbe for FanoutProjectArtifactRunner {}
    #[async_trait::async_trait]
    impl WorkflowStageRunner for FanoutProjectArtifactRunner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            assert_eq!(request.stage_kind, StageKind::Implementation);
            assert_eq!(
                request.input["target_repository_root"].as_str(),
                Some(self.project.display().to_string().as_str())
            );
            std::fs::create_dir_all(self.artifact.parent().unwrap()).unwrap();
            std::fs::write(&self.artifact, "{\"fanout\":true}").unwrap();
            Ok(implementation_evidence(
                "project-artifact-report",
                &[],
                &[self.artifact.to_str().unwrap()],
            ))
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let repo = temp.path().join("repo");
    let artifact = project.join(".archon/trading-lab/strategies/AHDM-v1/readiness/report.md");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(repo.join("Cargo.toml"), "[package]\nname = \"root-test\"\n").unwrap();

    let spec = WorkflowSpec::from_yaml(&format!(
        r#"
schema: archon.workflow.v1
name: fanout-project-artifact-root
task: Repair a project artifact through fanout while source work targets a repository.
target_repository_root: "{repo}"
stages:
  - id: repair_project_artifact
    kind: fanout
    item_kind: implementation
    input:
      items:
        - name: report
          task_id: project-artifact-report
    expected_target_files: ["{artifact}"]
"#,
        artifact = artifact.display(),
        repo = repo.display(),
    ))
    .unwrap();

    let store = WorkflowStore::project(&project);
    let executor = WorkflowExecutor::new(
        store.clone(),
        WorkflowPolicy {
            require_human_for_dangerous_tools: false,
            ..WorkflowPolicy::default()
        },
    );
    let run = executor.start(spec).unwrap();
    let run_id = run.id.clone();
    executor
        .execute_with_runner(
            run,
            &FanoutProjectArtifactRunner {
                project: project.clone(),
                artifact: artifact.clone(),
            },
        )
        .await
        .unwrap();

    let state = store.load_state(&run_id).unwrap();
    assert_eq!(
        state.stages.get("repair_project_artifact").unwrap().status,
        StageStatus::Accepted
    );
    assert!(artifact.exists());
}

#[tokio::test]
async fn implementation_without_repo_root_fails_run_fast() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    let spec = WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: missing-root
task: recovery continuation without embedded paths
stages:
  - id: implement
    kind: implementation
    task: Patch the recovered source file.
    required_work_units: [root-resolution]
    expected_target_files: ["src/lib.rs"]
  - id: should_not_run
    kind: agent
    depends_on: [implement]
"#,
    )
    .unwrap();

    let store = WorkflowStore::project(&project);
    let executor = WorkflowExecutor::new(
        store.clone(),
        WorkflowPolicy {
            require_human_for_dangerous_tools: false,
            ..WorkflowPolicy::default()
        },
    );
    let run = executor.start(spec).unwrap();
    let run_id = run.id.clone();
    let report = executor
        .execute_with_runner(run, &RootAssertingRunner { repo: project })
        .await
        .unwrap();

    let state = store.load_state(&run_id).unwrap();
    assert_eq!(report.failed, 1);
    assert_eq!(state.status, RunStatus::Failed);
    assert_eq!(
        state.stages.get("implement").unwrap().status,
        StageStatus::Failed
    );
    assert_eq!(
        state.stages.get("should_not_run").unwrap().status,
        StageStatus::Pending
    );
}
