//! Issue 255b with Issue 251: a host command's operational pause, inside the
//! executor lease, leaves the run `Paused` and the lease free. The next
//! resume then takes the paused path (no `stale_owner_recovered`) and the
//! interrupted call runs again.
//!
//! The executor here is this process, which stays alive after the pause, as
//! an interactive session does. So the lease can only be free because the
//! executor dropped it when the paused call returned, not because a process
//! ended.

use super::*;

use crate::command::workflow_host_command_catalog::{
    HostCommandResolutionContext, ResolvedHostCommand, fixed_decomposition_catalog,
};
use crate::command::workflow_host_command_exec::{
    FixedHostCommandExecutor, HostCommandProcessAdapter, WorkflowHostCommandExecutor,
};
use crate::command::workflow_host_command_supervisor::{
    HostCommandControl, SupervisedProcessOutput,
};
use archon_workflow::{HostCommandRequest, StageStatus, WorkflowError};

const TIME_OUT: usize = 0;
const EXIT_1: usize = 1;

/// Times out with no progress marker, or exits 1, by `mode`.
struct ModeProcess {
    mode: AtomicUsize,
    calls: AtomicUsize,
}

#[async_trait::async_trait]
impl HostCommandProcessAdapter for ModeProcess {
    async fn execute(
        &self,
        _request: ResolvedHostCommand,
        _control: HostCommandControl,
    ) -> archon_workflow::WorkflowResult<SupervisedProcessOutput> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let timed_out = self.mode.load(Ordering::SeqCst) == TIME_OUT;
        Ok(SupervisedProcessOutput {
            exit_code: (!timed_out).then_some(1),
            timed_out,
            stdout: Vec::new(),
            stderr: Vec::new(),
            stdout_bytes: 0,
            stderr_bytes: 0,
            stdout_retained_bytes: 0,
            stderr_retained_bytes: 0,
            stdout_truncated: false,
            stderr_truncated: false,
            stdout_path: None,
            stderr_path: None,
        })
    }
}

fn events(store: &WorkflowStore, run_id: &str) -> Vec<serde_json::Value> {
    std::fs::read_to_string(store.events_path(run_id))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// What an executor does once it holds the lease: records itself, marks the
/// run and the call's stage running, and dispatches one host command.
async fn execute_under_lease(
    store: &WorkflowStore,
    run_id: &str,
    executor: &FixedHostCommandExecutor,
) -> archon_workflow::WorkflowResult<archon_workflow::HostCommandResult> {
    let lease = crate::command::workflow_task_root_reclaim::begin_execution(store, run_id).unwrap();
    lease.record_executor().unwrap();
    let generation = store
        .with_run_lock(run_id, |locked| {
            let mut run = locked.load_state(run_id)?;
            run.status = RunStatus::Running;
            run.generation += 1;
            let mut stage = archon_workflow::run::StageState::pending("host-call");
            stage.status = StageStatus::Running;
            run.stages.insert("host-call".into(), stage);
            locked.save_state(&run)?;
            Ok(run.generation)
        })
        .unwrap();
    let request = HostCommandRequest::new("task-set-lint", None).unwrap();
    executor.execute(request, Some(generation)).await
}

#[tokio::test]
async fn an_operational_pause_releases_the_executor_lease_for_a_paused_resume() {
    let project = fixture_project();
    let root = project
        .path()
        .canonicalize()
        .map(archon_shell::paths::plain)
        .unwrap();
    let _ = run_fixed_decomposition_with_factory(
        project.path(),
        Path::new("prds/PRD-X.md"),
        Path::new("tasks/PRD-X"),
        None,
        true,
        &launch_config(project.path()),
        &empty_env(),
        &BarrierFactory::launch(root.clone()),
    )
    .await;
    let store = WorkflowStore::project(&root);
    let run_id = store.list_runs().unwrap().pop().unwrap().id;
    let prd_path = root.join("prds/PRD-X.md");
    let context = HostCommandResolutionContext {
        program: PathBuf::from("/trusted/archon"),
        project_root: root.clone(),
        prd_digest: archon_workflow::task_set_contract::content_digest(
            &std::fs::read(&prd_path).unwrap(),
        ),
        prd_path,
        task_root: root.join("tasks/PRD-X"),
        run_staging_root: store.run_dir(&run_id).join("host-command-staging"),
        frozen_task_id: None,
        frozen_task_file: None,
        freeze_provider_environment: Default::default(),
        gate_mode: GateMode::Observe,
        acceptance_environment_allowlist: Vec::new(),
    };
    let process = Arc::new(ModeProcess {
        mode: AtomicUsize::new(TIME_OUT),
        calls: AtomicUsize::new(0),
    });
    let executor = FixedHostCommandExecutor::with_process(
        fixed_decomposition_catalog("rev-1").unwrap(),
        context,
        store.run_dir(&run_id),
        process.clone(),
    );

    // No progress: one retry, then the executor pauses the run.
    let error = execute_under_lease(&store, &run_id, &executor)
        .await
        .unwrap_err();
    assert!(matches!(error, WorkflowError::ControlPaused(_)), "{error}");
    assert_eq!(process.calls.load(Ordering::SeqCst), 2);
    let run = store.load_state(&run_id).unwrap();
    assert_eq!(run.status, RunStatus::Paused);
    assert_eq!(run.stages["host-call"].status, StageStatus::Paused);
    let pauses: Vec<_> = events(&store, &run_id)
        .into_iter()
        .filter(|event| event["detail"]["event"] == "host_command_operational_pause")
        .collect();
    assert_eq!(pauses.len(), 1);
    assert_eq!(pauses[0]["detail"]["cause"], "no_progress_evidence");
    let lock = store.run_dir(&run_id).join("decomposition/executor.lock");
    let record: serde_json::Value = read_json(&lock);
    assert_eq!(record["role"], "executor");
    assert_eq!(record["holder"]["pid"], std::process::id());

    // The production resume takes the lease, sees `Paused`, records no
    // recovery, and reaches provider construction.
    let factory = BarrierFactory::resume(root.clone());
    let resume_error = resume_fixed_decomposition_with_factory(
        project.path(),
        &run_id,
        true,
        &launch_config(project.path()),
        &empty_env(),
        &factory,
    )
    .await
    .unwrap_err();
    let resume_error = format!("{resume_error:#}");
    assert!(
        resume_error.contains("barrier observed"),
        "the lease must be free after an operational pause: {resume_error}"
    );
    assert_eq!(factory.builds.load(Ordering::SeqCst), 1);
    assert!(
        events(&store, &run_id)
            .iter()
            .all(|event| event["kind"] != "stale_owner_recovered"),
        "a paused run is resumed, not recovered"
    );
    let record: serde_json::Value = read_json(&lock);
    assert_eq!(record["role"], "preflight");
    assert_eq!(record["last_executor"]["pid"], std::process::id());

    // The resume released the lease too; the next executor runs the
    // interrupted call again, and it completes.
    process.mode.store(EXIT_1, Ordering::SeqCst);
    let result = execute_under_lease(&store, &run_id, &executor)
        .await
        .unwrap();
    assert_eq!(result.exit_code, Some(1));
    assert_eq!(process.calls.load(Ordering::SeqCst), 3);
    assert_eq!(
        store.load_state(&run_id).unwrap().status,
        RunStatus::Running
    );
}
