//! Issue 337: the fixed executor obeys a run's recorded terminal stop. A
//! supervised process is signalled, a parent publication is refused and an
//! operational pause is refused for the generation the stop was taken at; a
//! stale stop (a lifecycle edit moved the generation) refuses nothing.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use archon_workflow::{HostCommandRequest, RunStatus, WorkflowError, WorkflowStore};

use super::workflow_host_command_catalog::{ResolvedHostCommand, fixed_decomposition_catalog};
use super::workflow_host_command_exec::{FixedHostCommandExecutor, WorkflowHostCommandExecutor};
use super::workflow_host_command_exec_tests::{PreparedBodyProcess, context, seed_frozen_chain};
use super::workflow_host_command_operational::{
    OperationalAttempt, OperationalReport, pause_for_stall, pause_run,
};
use super::workflow_host_command_supervisor::{
    HostCommandControl, HostCommandSignal, SupervisedProcessOutput,
};

fn new_run(store: &WorkflowStore) -> archon_workflow::WorkflowRun {
    store
        .create_run(archon_workflow::WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
            name: "terminal-stop".into(),
            task: "test".into(),
            target_repository_root: None,
            max_parallelism: 1,
            max_agents: 1,
            stages: Vec::new(),
            permissions: Default::default(),
            learning_hooks: Vec::new(),
        })
        .unwrap()
}

/// The record the script host writes under the run lock for a validated stop.
fn stop_at(store: &WorkflowStore, run_id: &str, generation: u64) {
    store
        .write_run_json(
            run_id,
            "v2/terminal-stop.json",
            &serde_json::json!({"generation": generation, "reason": "deliberate gate refusal"}),
        )
        .unwrap();
}

struct WaitingProcess {
    started: Arc<tokio::sync::Notify>,
}

#[async_trait::async_trait]
impl super::workflow_host_command_exec::HostCommandProcessAdapter for WaitingProcess {
    async fn execute(
        &self,
        request: ResolvedHostCommand,
        control: HostCommandControl,
    ) -> archon_workflow::WorkflowResult<SupervisedProcessOutput> {
        self.started.notify_one();
        Err(match control.wait().await {
            HostCommandSignal::Paused => WorkflowError::ControlPaused(format!(
                "host command '{}' paused while in flight",
                request.command_id
            )),
            HostCommandSignal::Cancelled => WorkflowError::ControlCancelled(format!(
                "host command '{}' cancelled while in flight",
                request.command_id
            )),
        })
    }
}

#[tokio::test]
async fn a_terminal_stop_signals_the_inflight_supervisor() {
    let temp = tempfile::tempdir().unwrap();
    let context = context(temp.path());
    let store = WorkflowStore::project(&context.project_root);
    let run = new_run(&store);
    let started = Arc::new(tokio::sync::Notify::new());
    let executor = Arc::new(FixedHostCommandExecutor::with_process(
        fixed_decomposition_catalog("rev-1").unwrap(),
        context,
        store.run_dir(&run.id),
        Arc::new(WaitingProcess {
            started: started.clone(),
        }),
    ));
    let generation = run.generation;
    let mut task = tokio::spawn(async move {
        executor
            .execute(
                HostCommandRequest::new("task-set-lint", None).unwrap(),
                Some(generation),
            )
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(2), started.notified())
        .await
        .expect("the process starts");

    // A stop another generation took is stale: the process keeps running.
    stop_at(&store, &run.id, generation + 7);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(400), &mut task)
            .await
            .is_err(),
        "a stale stop signals nothing"
    );
    // The stop of this generation ends it now, as a cancel of the call.
    stop_at(&store, &run.id, generation);
    let error = tokio::time::timeout(std::time::Duration::from_secs(2), task)
        .await
        .expect("the terminal stop reaches the supervisor")
        .unwrap()
        .unwrap_err();
    assert!(
        matches!(error, WorkflowError::ControlCancelled(_)),
        "{error:?}"
    );
    assert_ne!(store.load_state(&run.id).unwrap().status, RunStatus::Paused);
}

/// Records the stop while the process runs, then hands back a prepared,
/// auditable body: only the parent publication can still refuse it.
struct StopWhileRunning {
    inner: PreparedBodyProcess,
    store: WorkflowStore,
    run_id: String,
    stop_generation: Option<u64>,
    runs: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl super::workflow_host_command_exec::HostCommandProcessAdapter for StopWhileRunning {
    async fn execute(
        &self,
        request: ResolvedHostCommand,
        control: HostCommandControl,
    ) -> archon_workflow::WorkflowResult<SupervisedProcessOutput> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        if let Some(generation) = self.stop_generation {
            stop_at(&self.store, &self.run_id, generation);
        }
        self.inner.execute(request, control).await
    }
}

const CANDIDATE: &str = "# Candidate\n\n```yaml\ntask_id: TASK-X-010\ntitle: Candidate\ncomplexity: low\nstatus: ready\ndepends_on: []\nblocks: []\nimplements: []\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n```\n\n## Focused Tests\n- `test -f TASK-X-010.md`\n";

/// Lands one body. `stop` names the generation offset of a stop recorded
/// before the command (`Err`) or while it runs (`Ok`).
async fn land_body(
    stop: Option<Result<u64, u64>>,
) -> (archon_workflow::WorkflowResult<bool>, Vec<u8>, usize) {
    let temp = tempfile::tempdir().unwrap();
    let mut context = context(temp.path());
    let task_file = context.task_root.join("TASK-X-010.md");
    std::fs::write(&task_file, b"live-before").unwrap();
    seed_frozen_chain(&context, &task_file);
    let store = WorkflowStore::project(&context.project_root);
    let run = new_run(&store);
    let run_root = store.run_dir(&run.id);
    context.run_staging_root = run_root.join("host-command-staging");
    let stop_generation = match stop {
        Some(Err(offset)) => {
            stop_at(&store, &run.id, run.generation + offset);
            None
        }
        Some(Ok(offset)) => Some(run.generation + offset),
        None => None,
    };
    let runs = Arc::new(AtomicUsize::new(0));
    let executor = FixedHostCommandExecutor::with_process(
        fixed_decomposition_catalog("rev-1").unwrap(),
        context,
        run_root,
        Arc::new(StopWhileRunning {
            inner: PreparedBodyProcess {
                candidate: CANDIDATE.as_bytes().to_vec(),
            },
            store: store.clone(),
            run_id: run.id.clone(),
            stop_generation,
            runs: runs.clone(),
        }),
    );
    let request = HostCommandRequest::new("land-task-body", Some(CANDIDATE.to_string())).unwrap();
    let outcome = executor
        .execute(request, Some(run.generation))
        .await
        .map(|result| result.publication_receipt.is_some());
    (
        outcome,
        std::fs::read(&task_file).unwrap(),
        runs.load(Ordering::SeqCst),
    )
}

#[tokio::test]
async fn a_terminal_stop_refuses_a_sibling_publication() {
    // Recorded while the body ran: the parent publication refuses it.
    let (outcome, live, runs) = land_body(Some(Ok(0))).await;
    assert!(
        matches!(&outcome, Err(WorkflowError::ControlCancelled(message)) if message.contains("stopped terminally")),
        "{outcome:?}"
    );
    assert_eq!(live, b"live-before", "nothing published after the stop");
    assert_eq!(runs, 1);
    // Recorded before it started: the command never runs.
    let (outcome, live, runs) = land_body(Some(Err(0))).await;
    assert!(
        matches!(outcome, Err(WorkflowError::ControlCancelled(_))),
        "{outcome:?}"
    );
    assert_eq!((live.as_slice(), runs), (&b"live-before"[..], 0));
    // A stale stop refuses nothing: the body lands.
    let (outcome, live, _) = land_body(Some(Ok(3))).await;
    assert!(matches!(outcome, Ok(true)), "{outcome:?}");
    assert_eq!(live, CANDIDATE.as_bytes());
}

#[test]
fn a_terminal_stop_refuses_the_operational_pauses_of_its_generation() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let run = new_run(&store);
    let mut running = store.load_state(&run.id).unwrap();
    running.status = RunStatus::Running;
    store.save_state(&running).unwrap();
    let run_root = store.run_dir(&run.id);
    let attempts = [OperationalAttempt {
        attempt: 3,
        reason: "timeout",
        elapsed_secs: 9,
        progress: None,
    }];
    let report = OperationalReport {
        run_id: &run.id,
        call_id: "host-command:task-set-lint:fixed",
        command_id: "task-set-lint",
        limit_secs: 3,
        attempts: &attempts,
    };
    stop_at(&store, &run.id, running.generation);

    for refused in [
        pause_run(
            &store,
            &run_root,
            running.generation,
            &report,
            "no_progress",
        ),
        pause_for_stall(&store, &run_root, running.generation, &report, "kept group"),
    ] {
        assert!(
            matches!(&refused, WorkflowError::ControlCancelled(message) if message.contains("stopped terminally")),
            "{refused:?}"
        );
        let after = store.load_state(&run.id).unwrap();
        assert_eq!(
            (after.status, after.generation),
            (RunStatus::Running, running.generation)
        );
    }

    // An operator's pause and resume made the stop stale: the pause is taken.
    let lifecycle = archon_workflow::LifecycleController::new(store.clone());
    lifecycle
        .apply(&run.id, archon_workflow::LifecycleAction::Pause)
        .unwrap();
    lifecycle
        .apply(&run.id, archon_workflow::LifecycleAction::Resume)
        .unwrap();
    let resumed = store.load_state(&run.id).unwrap().generation;
    let paused = pause_run(&store, &run_root, resumed, &report, "no_progress");
    assert!(
        matches!(paused, WorkflowError::ControlPaused(_)),
        "{paused:?}"
    );
    assert_eq!(store.load_state(&run.id).unwrap().status, RunStatus::Paused);
}

fn write_unreadable_stop(store: &WorkflowStore, run_id: &str) {
    let dir = store.run_dir(run_id).join("v2");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("terminal-stop.json"), b"{").unwrap();
}

fn set_aside(store: &WorkflowStore, run_id: &str) -> Vec<String> {
    std::fs::read_dir(store.run_dir(run_id).join("v2"))
        .unwrap()
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| name.starts_with("terminal-stop.json.unreadable"))
        .collect()
}

fn pause_events(store: &WorkflowStore, run_id: &str) -> usize {
    std::fs::read_to_string(store.events_path(run_id))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|event| event["kind"] == "paused")
        .count()
}

/// Round 3 (review finding 5): a stop record that cannot be read may be a
/// stop. The writer never continues past it: the run pauses once with that
/// evidence, and the record is set aside (kept) so a resume is not refused
/// again and no later reader reports it again.
#[test]
fn an_unreadable_stop_record_pauses_the_writer_once_and_is_set_aside() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let run = new_run(&store);
    let mut running = store.load_state(&run.id).unwrap();
    running.status = RunStatus::Running;
    store.save_state(&running).unwrap();
    write_unreadable_stop(&store, &run.id);

    let first = super::workflow_host_command_operational::require_run_owned(
        &store,
        &run.id,
        running.generation,
    );
    assert!(
        matches!(&first, Err(WorkflowError::ControlPaused(message)) if message.contains("terminal stop record")),
        "{first:?}"
    );
    assert_eq!(store.load_state(&run.id).unwrap().status, RunStatus::Paused);
    assert!(
        !store
            .run_dir(&run.id)
            .join("v2/terminal-stop.json")
            .exists()
    );
    assert_eq!(set_aside(&store, &run.id).len(), 1, "kept as evidence");

    let again = super::workflow_host_command_operational::require_run_owned(
        &store,
        &run.id,
        running.generation,
    );
    assert!(
        matches!(again, Err(WorkflowError::ControlPaused(_))),
        "{again:?}"
    );
    assert_eq!(pause_events(&store, &run.id), 1, "one pause, one report");
}

#[tokio::test]
async fn an_unreadable_stop_record_pauses_the_inflight_supervisor() {
    let temp = tempfile::tempdir().unwrap();
    let context = context(temp.path());
    let store = WorkflowStore::project(&context.project_root);
    let run = new_run(&store);
    let started = Arc::new(tokio::sync::Notify::new());
    let executor = Arc::new(FixedHostCommandExecutor::with_process(
        fixed_decomposition_catalog("rev-1").unwrap(),
        context,
        store.run_dir(&run.id),
        Arc::new(WaitingProcess {
            started: started.clone(),
        }),
    ));
    let generation = run.generation;
    let task = tokio::spawn(async move {
        executor
            .execute(
                HostCommandRequest::new("task-set-lint", None).unwrap(),
                Some(generation),
            )
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(2), started.notified())
        .await
        .expect("the process starts");
    write_unreadable_stop(&store, &run.id);

    let error = tokio::time::timeout(std::time::Duration::from_secs(2), task)
        .await
        .expect("an unreadable stop record reaches the supervisor")
        .unwrap()
        .unwrap_err();
    assert!(
        matches!(error, WorkflowError::ControlPaused(_)),
        "{error:?}"
    );
    assert_eq!(store.load_state(&run.id).unwrap().status, RunStatus::Paused);
    assert_eq!(pause_events(&store, &run.id), 1);
}
