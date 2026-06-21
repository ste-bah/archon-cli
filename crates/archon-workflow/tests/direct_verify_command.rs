use archon_workflow::{
    LifecycleAction, LifecycleController, RunStatus, StageRunOutput, StageRunRequest, StageStatus,
    WorkflowExecutor, WorkflowPolicy, WorkflowSpec, WorkflowStageRunner, WorkflowStore,
};
use serde_json::json;

struct PanicRunner;

impl archon_workflow::WriteBoundaryProbe for PanicRunner {}

#[async_trait::async_trait]
impl WorkflowStageRunner for PanicRunner {
    async fn run_stage(
        &self,
        request: StageRunRequest,
    ) -> archon_workflow::WorkflowResult<StageRunOutput> {
        panic!("local verify_command stage delegated to runner: {request:?}");
    }
}

fn permissive_policy() -> WorkflowPolicy {
    WorkflowPolicy {
        require_human_for_dangerous_tools: false,
        ..WorkflowPolicy::default()
    }
}

fn repo_root(parent: &std::path::Path) -> std::path::PathBuf {
    let repo = parent.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(
        repo.join("Cargo.toml"),
        "[package]\nname='verify-root'\nversion='0.0.0'\n",
    )
    .unwrap();
    repo
}

fn direct_verify_spec(repo: &std::path::Path, command: &str) -> WorkflowSpec {
    WorkflowSpec::from_yaml(&format!(
        r#"
schema: archon.workflow.v1
name: direct-local-verify
task: Run the exact verification command.
target_repository_root: "{}"
stages:
  - id: focused_tests
    kind: agent
    provider_tier: local
    verify_command: '{}'
"#,
        repo.display(),
        command.replace('\'', "''")
    ))
    .unwrap()
}

#[tokio::test]
async fn local_agent_verify_command_runs_directly() {
    let temp = tempfile::tempdir().unwrap();
    let repo = repo_root(temp.path());
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let spec = direct_verify_spec(&repo, "printf verified > direct.out");

    let run = executor.start(spec).unwrap();
    let report = executor
        .execute_with_runner(run.clone(), &PanicRunner)
        .await
        .unwrap();

    assert_eq!(report.failed, 0);
    assert_eq!(
        std::fs::read_to_string(repo.join("direct.out")).unwrap(),
        "verified"
    );
    let finished = store.load_state(&run.id).unwrap();
    assert_eq!(
        finished.stages.get("focused_tests").unwrap().status,
        StageStatus::Accepted
    );
    let command_record = store
        .run_dir(&run.id)
        .join("command-executions/focused_tests/cmd-0001.json");
    let record: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(command_record).unwrap()).unwrap();
    assert_eq!(record["role"], "verification");
    assert_eq!(record["status"], "completed");
    assert_eq!(record["command"], "printf verified > direct.out");
    assert!(record["process_group"].as_u64().is_some());
    assert_eq!(record["progress_class"], "unknown_progress");
}

#[tokio::test]
async fn local_agent_verify_command_failure_fails_stage() {
    let temp = tempfile::tempdir().unwrap();
    let repo = repo_root(temp.path());
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let spec = direct_verify_spec(&repo, "printf nope >&2; exit 7");

    let run = executor.start(spec).unwrap();
    let report = executor
        .execute_with_runner(run.clone(), &PanicRunner)
        .await
        .unwrap();

    assert_eq!(report.failed, 1);
    let finished = store.load_state(&run.id).unwrap();
    assert_eq!(finished.status, RunStatus::Failed);
    let state = finished.stages.get("focused_tests").unwrap();
    assert_eq!(state.status, StageStatus::Failed);
    assert!(
        state
            .error
            .as_deref()
            .is_some_and(|reason| reason.contains('7'))
    );
}

#[tokio::test]
async fn local_agent_verify_command_list_only_fails_stage() {
    let temp = tempfile::tempdir().unwrap();
    let repo = repo_root(temp.path());
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let spec = direct_verify_spec(&repo, "printf listed; true --list");

    let run = executor.start(spec).unwrap();
    let report = executor
        .execute_with_runner(run.clone(), &PanicRunner)
        .await
        .unwrap();

    assert_eq!(report.failed, 1);
    let finished = store.load_state(&run.id).unwrap();
    let state = finished.stages.get("focused_tests").unwrap();
    assert_eq!(state.status, StageStatus::Failed);
    assert!(
        state
            .error
            .as_deref()
            .is_some_and(|reason| reason.contains("discovery/list-only"))
    );
}

#[tokio::test]
async fn local_agent_verify_command_zero_work_output_fails_stage() {
    let temp = tempfile::tempdir().unwrap();
    let repo = repo_root(temp.path());
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let spec = direct_verify_spec(&repo, "printf 'running 0 tests\\n'");

    let run = executor.start(spec).unwrap();
    let report = executor
        .execute_with_runner(run.clone(), &PanicRunner)
        .await
        .unwrap();

    assert_eq!(report.failed, 1);
    let finished = store.load_state(&run.id).unwrap();
    let state = finished.stages.get("focused_tests").unwrap();
    assert_eq!(state.status, StageStatus::Failed);
    assert!(
        state
            .error
            .as_deref()
            .is_some_and(|reason| reason.contains("zero-test/no-op"))
    );
}

#[tokio::test]
async fn local_agent_verify_command_rejects_unresolved_workflow_template() {
    let temp = tempfile::tempdir().unwrap();
    let repo = repo_root(temp.path());
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let spec = direct_verify_spec(&repo, "${previous_stage.focused_test_command}");

    let run = executor.start(spec).unwrap();
    let report = executor
        .execute_with_runner(run.clone(), &PanicRunner)
        .await
        .unwrap();

    assert_eq!(report.failed, 1);
    let finished = store.load_state(&run.id).unwrap();
    let state = finished.stages.get("focused_tests").unwrap();
    assert_eq!(state.status, StageStatus::Failed);
    assert!(
        state
            .error
            .as_deref()
            .is_some_and(|reason| reason.contains("unresolved workflow template"))
    );
}

#[tokio::test]
async fn local_agent_verify_command_emits_stall_event() {
    let temp = tempfile::tempdir().unwrap();
    let repo = repo_root(temp.path());
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let spec = WorkflowSpec::from_yaml(&format!(
        r#"
schema: archon.workflow.v1
name: direct-local-verify-stall
task: Run the exact verification command.
target_repository_root: "{}"
stages:
  - id: focused_tests
    kind: agent
    provider_tier: local
    verify_command: 'sleep 1; printf done'
    command_stall_after_secs: 0
"#,
        repo.display()
    ))
    .unwrap();

    let run = executor.start(spec).unwrap();
    let report = executor
        .execute_with_runner(run.clone(), &PanicRunner)
        .await
        .unwrap();

    assert_eq!(report.failed, 0);
    let events = std::fs::read_to_string(store.events_path(&run.id)).unwrap();
    assert!(events.contains(r#""kind":"stage_stalled""#), "{events}");
    let command_record = store
        .run_dir(&run.id)
        .join("command-executions/focused_tests/cmd-0001.json");
    let record: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(command_record).unwrap()).unwrap();
    assert_eq!(record["status"], "completed");
}

#[cfg(unix)]
#[test]
fn lifecycle_cancel_terminates_recorded_process_group() {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    use std::time::Duration;

    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let spec = direct_verify_spec(temp.path(), "true");
    let run = executor.start(spec).unwrap();

    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg("sleep 20")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    unsafe {
        command.pre_exec(|| {
            if libc::setpgid(0, 0) == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        });
    }
    let mut child = command.spawn().unwrap();
    let pgid = child.id();
    store
        .write_run_json(
            &run.id,
            "command-executions/focused_tests/cmd-0001.json",
            &json!({
                "schema": "archon.workflow.command_execution.v1",
                "run_id": run.id,
                "stage_id": "focused_tests",
                "attempt_id": "focused_tests-attempt-1",
                "command_id": "cmd-0001",
                "role": "verification",
                "command": "sleep 20",
                "cwd": temp.path(),
                "process_group": pgid,
                "started_at": chrono::Utc::now().to_rfc3339(),
                "last_output_at": null,
                "last_progress_at": chrono::Utc::now().to_rfc3339(),
                "progress_class": "unknown_progress",
                "status": "running",
                "exit_status": null
            }),
        )
        .unwrap();

    let cancelled = LifecycleController::new(store.clone())
        .apply(&run.id, LifecycleAction::Cancel)
        .unwrap();

    assert_eq!(cancelled.status, RunStatus::Cancelled);
    for _ in 0..20 {
        if child.try_wait().unwrap().is_some() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("recorded workflow-owned process group was not terminated");
}
