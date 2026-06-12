use std::path::PathBuf;

use archon_workflow::{
    RunStatus, StageKind, StageRunOutput, StageRunRequest, StageStatus, WorkflowExecutor,
    WorkflowPolicy, WorkflowSpec, WorkflowStageRunner, WorkflowStore,
};

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
        std::fs::write(self.repo.join("src/lib.rs"), "pub fn implemented() {}").unwrap();
        Ok(StageRunOutput::markdown("implemented from recovered root"))
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
