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
