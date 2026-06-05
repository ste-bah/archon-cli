//! Phase B tests for PRD-009 write-capable implementation stages.
//!
//! Acceptance binding contract: an `implementation` stage is accepted ONLY when
//! every `expected_target_files` entry is mutated AND the `verify_command`
//! exits 0. Otherwise the stage fails and its artifact is recorded as
//! not-accepted. The stage is also write-gated by policy.

use std::path::PathBuf;

use archon_workflow::{
    PolicyDecision, StageKind, StageRunOutput, StageRunRequest, StageStatus, WorkflowExecutor,
    WorkflowPolicy, WorkflowSpec, WorkflowStageRunner, WorkflowStore,
};

/// A runner that optionally writes `content` to `target` (an absolute path)
/// to simulate a write-capable agent mutating its declared target file.
struct ImplRunner {
    target: PathBuf,
    content: Option<String>,
}

#[async_trait::async_trait]
impl WorkflowStageRunner for ImplRunner {
    async fn run_stage(
        &self,
        request: StageRunRequest,
    ) -> archon_workflow::WorkflowResult<StageRunOutput> {
        if let Some(content) = &self.content {
            std::fs::write(&self.target, content).unwrap();
        }
        Ok(StageRunOutput::markdown(format!(
            "implemented {}",
            request.stage_id
        )))
    }
}

fn impl_spec(target: &str, verify_command: Option<&str>) -> WorkflowSpec {
    let verify = verify_command
        .map(|cmd| format!("    verify_command: \"{cmd}\"\n"))
        .unwrap_or_default();
    WorkflowSpec::from_yaml(&format!(
        r#"
schema: archon.workflow.v1
name: phase-b-test
task: write-capable implementation coverage
stages:
  - id: implement
    kind: implementation
    agent: workflow-coder
    expected_target_files:
      - "{target}"
{verify}"#,
    ))
    .unwrap()
}

/// Policy that permits implementation stages to run (write gate disabled).
fn permissive_policy() -> WorkflowPolicy {
    WorkflowPolicy {
        require_human_for_dangerous_tools: false,
        ..WorkflowPolicy::default()
    }
}

#[tokio::test]
async fn implementation_accepted_when_target_mutated_and_verify_passes() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("out.txt");
    std::fs::write(&target, "before").unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let run = executor
        .start(impl_spec(target.to_str().unwrap(), Some("true")))
        .unwrap();
    let run_id = run.id.clone();
    let runner = ImplRunner {
        target: target.clone(),
        content: Some("after".into()),
    };
    executor.execute_with_runner(run, &runner).await.unwrap();

    let state = store.load_state(&run_id).unwrap();
    let stage = state.stages.get("implement").unwrap();
    assert_eq!(
        stage.status,
        StageStatus::Accepted,
        "stage must be accepted"
    );
    assert!(
        stage.artifacts.iter().any(|a| a.accepted),
        "an accepted artifact must exist"
    );
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "after");
}

#[tokio::test]
async fn implementation_rejected_when_target_not_mutated() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("out.txt");
    std::fs::write(&target, "before").unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let run = executor
        .start(impl_spec(target.to_str().unwrap(), Some("true")))
        .unwrap();
    let run_id = run.id.clone();
    // content: None -> runner does NOT touch the target file.
    let runner = ImplRunner {
        target: target.clone(),
        content: None,
    };
    // Stage failures are recorded as StageStatus::Failed (not propagated as Err).
    executor.execute_with_runner(run, &runner).await.unwrap();

    let state = store.load_state(&run_id).unwrap();
    let stage = state.stages.get("implement").unwrap();
    assert_eq!(
        stage.status,
        StageStatus::Failed,
        "unmutated target must reject the stage"
    );
    assert!(
        stage.artifacts.iter().all(|a| !a.accepted),
        "artifact must be recorded not-accepted"
    );
}

#[tokio::test]
async fn implementation_rejected_when_verify_command_fails() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("out.txt");
    std::fs::write(&target, "before").unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let run = executor
        .start(impl_spec(target.to_str().unwrap(), Some("exit 1")))
        .unwrap();
    let run_id = run.id.clone();
    // Target IS mutated, but verify_command fails -> rejected.
    let runner = ImplRunner {
        target: target.clone(),
        content: Some("after".into()),
    };
    executor.execute_with_runner(run, &runner).await.unwrap();

    let state = store.load_state(&run_id).unwrap();
    let stage = state.stages.get("implement").unwrap();
    assert_eq!(
        stage.status,
        StageStatus::Failed,
        "failing verify_command must reject the stage"
    );
    assert!(
        stage.artifacts.iter().all(|a| !a.accepted),
        "artifact must be recorded not-accepted when verify fails"
    );
}

#[test]
fn implementation_spec_requires_expected_target_files() {
    let err = WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: phase-b-invalid
task: missing targets
stages:
  - id: implement
    kind: implementation
    agent: workflow-coder
"#,
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("expected_target_files"),
        "error should mention expected_target_files: {err}"
    );
}

#[test]
fn implementation_is_write_gated_by_default_policy() {
    let spec = impl_spec("out.txt", None);
    let stage = spec
        .stages
        .iter()
        .find(|s| s.kind == StageKind::Implementation)
        .unwrap();
    // Default policy requires human approval for write-capable stages.
    let decision = WorkflowPolicy::default().stage_decision(stage);
    assert!(
        matches!(decision, PolicyDecision::RequireHuman(_)),
        "default policy must gate implementation stages: {decision:?}"
    );
    // Permissive policy (operator opt-in) allows it.
    let allowed = permissive_policy().stage_decision(stage);
    assert_eq!(allowed, PolicyDecision::Allow);
}

#[tokio::test]
async fn implementation_start_denied_under_default_policy() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("out.txt");
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store, WorkflowPolicy::default());
    let err = executor
        .start(impl_spec(target.to_str().unwrap(), None))
        .unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("policy"),
        "default policy should deny start: {err}"
    );
}

#[tokio::test]
async fn implementation_fanout_mutates_each_item_target() {
    struct FanoutImplRunner;

    #[async_trait::async_trait]
    impl WorkflowStageRunner for FanoutImplRunner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            assert_eq!(request.stage_kind, StageKind::Implementation);
            let target = request.input["fanout_item"]["target_files"][0]
                .as_str()
                .unwrap();
            std::fs::write(target, format!("changed by {}", request.stage_id)).unwrap();
            Ok(StageRunOutput::markdown("implemented fanout item"))
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("fanout.txt");
    std::fs::write(&target, "before").unwrap();
    let spec = WorkflowSpec::from_yaml(&format!(
        r#"
schema: archon.workflow.v1
name: fanout-implementation
task: implement each item
stages:
  - id: implement_task
    kind: fanout
    item_kind: implementation
    provider_tier: coder
    input:
      items:
        - task_id: T001
          target_files: ["{}"]
"#,
        target.display()
    ))
    .unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let run = executor.start(spec).unwrap();
    let report = executor
        .execute_with_runner(run.clone(), &FanoutImplRunner)
        .await
        .unwrap();
    assert_eq!(report.failed, 0);

    let state = store.load_state(&run.id).unwrap();
    assert_eq!(
        state.stages.get("implement_task").unwrap().status,
        StageStatus::Accepted
    );
    assert!(
        std::fs::read_to_string(&target)
            .unwrap()
            .contains("changed")
    );
}

#[tokio::test]
async fn legacy_implementation_fanout_without_item_kind_still_writes() {
    struct LegacyImplRunner;

    #[async_trait::async_trait]
    impl WorkflowStageRunner for LegacyImplRunner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            assert_eq!(request.stage_kind, StageKind::Implementation);
            let target = request.input["fanout_item"]["target_files"][0]
                .as_str()
                .unwrap();
            std::fs::write(target, "legacy fanout changed").unwrap();
            Ok(StageRunOutput::markdown("implemented legacy fanout item"))
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("legacy-fanout.txt");
    std::fs::write(&target, "before").unwrap();
    let spec = WorkflowSpec::from_yaml(&format!(
        r#"
schema: archon.workflow.v1
name: legacy-fanout-implementation
task: implement each item
stages:
  - id: implement_task
    kind: fanout
    item_kind: implementation
    task: Implement only the missing work for each item.
    provider_tier: coder
    input:
      items:
        - task_id: T001
          target_files: ["{}"]
"#,
        target.display()
    ))
    .unwrap();

    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let mut run = executor.start(spec).unwrap();
    run.spec.stages[0].item_kind = None;
    let report = executor
        .execute_with_runner(run.clone(), &LegacyImplRunner)
        .await
        .unwrap();
    assert_eq!(report.failed, 0);
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "legacy fanout changed"
    );
}

#[tokio::test]
async fn implementation_fanout_gets_target_repo_sources_and_greenfield_targets() {
    struct EvidenceRunner {
        repo: PathBuf,
        existing_target: PathBuf,
        new_target: PathBuf,
        task_file: PathBuf,
    }

    #[async_trait::async_trait]
    impl WorkflowStageRunner for EvidenceRunner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            if request.stage_id == "inventory" {
                return Ok(StageRunOutput::markdown("inventory complete"));
            }

            let root = request.input["target_repository_root"].as_str().unwrap();
            assert_eq!(root, self.repo.display().to_string());

            let sources = request.input["source_files"].as_array().unwrap();
            let existing_target = self.existing_target.canonicalize().unwrap();
            let task_file = self.task_file.canonicalize().unwrap();
            assert!(
                sources.iter().any(|source| {
                    source["absolute_path"].as_str() == Some(existing_target.to_str().unwrap())
                        && source["content"].as_str().unwrap().contains("before")
                }),
                "existing target file content should be attached: {sources:#?}"
            );
            assert!(
                sources.iter().any(|source| {
                    source["absolute_path"].as_str() == Some(task_file.to_str().unwrap())
                        && source["content"].as_str().unwrap().contains("REQ-TEST")
                }),
                "task file evidence should be attached: {sources:#?}"
            );
            assert!(
                sources.iter().any(|source| {
                    source["absolute_path"].as_str() == Some(self.new_target.to_str().unwrap())
                        && source["exists"].as_bool() == Some(false)
                }),
                "greenfield target should be attached as exists:false: {sources:#?}"
            );

            std::fs::write(&self.existing_target, "after").unwrap();
            std::fs::write(&self.new_target, "new").unwrap();
            Ok(StageRunOutput::markdown("implemented with evidence"))
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let repo = temp.path().join("repo");
    let task_dir = project.join("tasks");
    std::fs::create_dir_all(&task_dir).unwrap();
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    std::fs::create_dir_all(repo.join("src")).unwrap();

    let existing_target = repo.join("Cargo.toml");
    let new_target = repo.join("src").join("new.rs");
    let task_file = task_dir.join("TASK.md");
    std::fs::write(&existing_target, "before").unwrap();
    std::fs::write(&task_file, "REQ-TEST: implement the greenfield module").unwrap();

    let spec = WorkflowSpec::from_yaml(&format!(
        r#"
schema: archon.workflow.v1
name: target-repo-source-evidence
task: "Implement {task_file} against repository {repo}"
stages:
  - id: inventory
    kind: agent
    outputs: [items]
  - id: implement_task
    kind: fanout
    item_kind: implementation
    provider_tier: coder
    depends_on: [inventory]
    input:
      items:
        - task_id: T001
          task_file: "{task_file}"
          target_files: ["Cargo.toml", "src/new.rs"]
"#,
        task_file = task_file.display(),
        repo = repo.display()
    ))
    .unwrap();

    let store = WorkflowStore::project(&project);
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let run = executor.start(spec).unwrap();
    let report = executor
        .execute_with_runner(
            run.clone(),
            &EvidenceRunner {
                repo,
                existing_target,
                new_target,
                task_file,
            },
        )
        .await
        .unwrap();
    assert_eq!(report.failed, 0);

    let state = store.load_state(&run.id).unwrap();
    assert_eq!(
        state.stages.get("implement_task").unwrap().status,
        StageStatus::Accepted
    );
}

#[test]
fn implementation_fanout_is_write_gated_by_default_policy() {
    let spec = WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: fanout-implementation-gate
task: implement each item
stages:
  - id: implement_task
    kind: fanout
    item_kind: implementation
    input:
      items:
        - target_files: ["out.txt"]
"#,
    )
    .unwrap();
    let stage = spec
        .stages
        .iter()
        .find(|s| s.id == "implement_task")
        .unwrap();
    let decision = WorkflowPolicy::default().stage_decision(stage);
    assert!(
        matches!(decision, PolicyDecision::RequireHuman(_)),
        "implementation fanout must be write gated: {decision:?}"
    );
}
