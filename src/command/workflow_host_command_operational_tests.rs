//! Issue #255: an operational ending of a host command is retried while it
//! makes progress and then pauses the run; it never fails the work.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use archon_workflow::{HostCommandRequest, RunStatus, StageStatus, WorkflowError, WorkflowStore};

use super::*;
use crate::command::workflow_host_command_catalog::{
    HostCommandResolutionContext, ResolvedHostCommand, fixed_decomposition_catalog,
};
use crate::command::workflow_host_command_exec::{
    FixedHostCommandExecutor, HostCommandProcessAdapter, WorkflowHostCommandExecutor,
};
use crate::command::workflow_host_command_exec_tests::{
    PreparedBodyProcess, context, seed_frozen_chain,
};
use crate::command::workflow_host_command_supervisor::HostCommandControl;

fn attempt(n: u32, progress: Option<u64>) -> OperationalAttempt {
    OperationalAttempt::new(n, "timed_out", 1, progress)
}

fn output(exit_code: Option<i32>, timed_out: bool, stderr: &str) -> SupervisedProcessOutput {
    SupervisedProcessOutput {
        exit_code,
        timed_out,
        stdout: Vec::new(),
        stderr: stderr.as_bytes().to_vec(),
        stdout_bytes: 0,
        stderr_bytes: stderr.len() as u64,
        stdout_retained_bytes: 0,
        stderr_retained_bytes: stderr.len() as u64,
        stdout_truncated: false,
        stderr_truncated: false,
        stdout_path: None,
        stderr_path: None,
    }
}

#[test]
fn the_contract_classifies_only_timeouts_and_the_resumable_exit() {
    assert_eq!(
        classify(&output(None, true, "")),
        Some(OperationalKind::TimedOut)
    );
    assert_eq!(
        classify(&output(Some(EXIT_INCOMPLETE_RESUMABLE), false, "")),
        Some(OperationalKind::IncompleteResumable)
    );
    for code in [Some(0), Some(1), Some(2), None] {
        assert_eq!(classify(&output(code, false, "")), None, "{code:?}");
    }
    let stderr = format!(
        "noise\n{}\nmore\n  {}  \n",
        progress_line(3),
        progress_line(7)
    );
    assert_eq!(reported_progress(stderr.as_bytes()), Some(7));
    assert_eq!(reported_progress(b"archon-host-progress: many\n"), None);
    assert_eq!(reported_progress(b""), None);
}

#[test]
fn retries_need_growing_progress() {
    use NextStep::{Pause, Retry};
    // No marker retries once; growing progress keeps retrying.
    assert_eq!(next_step(&[attempt(1, None)]), Retry);
    assert_eq!(
        next_step(&[attempt(1, None), attempt(2, None)]),
        Pause("no_progress_evidence")
    );
    assert_eq!(next_step(&[attempt(1, Some(1))]), Retry);
    let growing = |n: u32| -> Vec<OperationalAttempt> {
        (1..=n).map(|i| attempt(i, Some(u64::from(i)))).collect()
    };
    assert_eq!(next_step(&growing(10)), Retry);
    assert_eq!(next_step(&growing(130)), Retry);
    assert_eq!(next_step(&[attempt(1, Some(0))]), Retry);
    assert_eq!(
        next_step(&[attempt(1, Some(0)), attempt(2, Some(0))]),
        Pause("no_progress")
    );
    assert_eq!(
        next_step(&[attempt(1, None), attempt(2, Some(3))]),
        Pause("no_progress_evidence")
    );
    assert_eq!(
        next_step(&[attempt(1, Some(4)), attempt(2, Some(4))]),
        Pause("no_progress")
    );
    assert_eq!(
        next_step(&[attempt(1, Some(4)), attempt(2, None)]),
        Pause("no_progress_evidence")
    );
}

struct ScriptedProcess {
    calls: AtomicUsize,
    spill_dirs: Mutex<Vec<std::path::PathBuf>>,
    outcomes: Vec<Scripted>,
    run: (WorkflowStore, String),
}

#[derive(Clone)]
enum Scripted {
    TimedOut(Option<u64>),
    Incomplete(u64),
    Exit(i32),
    Error(&'static str),
    Publish(Vec<u8>),
    OperatorPauseThenTimeOut,
    Stderr(i32, String),
    TransientStderr(String),
}

#[async_trait::async_trait]
impl HostCommandProcessAdapter for ScriptedProcess {
    async fn execute(
        &self,
        request: ResolvedHostCommand,
        control: HostCommandControl,
    ) -> WorkflowResult<SupervisedProcessOutput> {
        request
            .spill_dir
            .as_ref()
            .map(|directory| self.spill_dirs.lock().unwrap().push(directory.clone()));
        let index = self.calls.fetch_add(1, Ordering::SeqCst);
        let scripted = self.outcomes[index.min(self.outcomes.len() - 1)].clone();
        // A retry must clear partial staging before auditing.
        let stale = request.declared_write_set[0].with_file_name("partial-leftover");
        std::fs::create_dir_all(stale.parent().unwrap()).unwrap();
        let marker = |p: Option<u64>| p.map(progress_line).unwrap_or_default();
        match scripted {
            Scripted::TimedOut(progress) => {
                std::fs::write(stale, b"partial").unwrap();
                Ok(output(None, true, &marker(progress)))
            }
            Scripted::Incomplete(progress) => {
                std::fs::write(stale, b"partial").unwrap();
                let stderr = marker(Some(progress));
                let mut outcome = output(Some(EXIT_INCOMPLETE_RESUMABLE), false, &stderr);
                outcome.stdout = serde_json::to_vec(&archon_workflow::GateOperationalError {
                    kind: "transient_host_fault".into(),
                    text: "the observation window elapsed".into(),
                })
                .unwrap();
                outcome.stdout_bytes = outcome.stdout.len() as u64;
                outcome.stdout_retained_bytes = outcome.stdout_bytes;
                Ok(outcome)
            }
            Scripted::TransientStderr(stderr) => {
                let mut outcome = output(Some(EXIT_INCOMPLETE_RESUMABLE), false, &stderr);
                outcome.stdout = serde_json::to_vec(&archon_workflow::GateOperationalError {
                    kind: "transient_host_fault".into(),
                    text: "the host observation was interrupted".into(),
                })
                .unwrap();
                outcome.stdout_bytes = outcome.stdout.len() as u64;
                outcome.stdout_retained_bytes = outcome.stdout_bytes;
                Ok(outcome)
            }
            Scripted::Exit(code) => Ok(output(Some(code), false, "genuine failure")),
            Scripted::Stderr(code, stderr) => Ok(output(Some(code), false, &stderr)),
            Scripted::Error(text) => Err(WorkflowError::StageFailed(text.into())),
            Scripted::OperatorPauseThenTimeOut => {
                archon_workflow::LifecycleController::new(self.run.0.clone())
                    .apply(&self.run.1, archon_workflow::LifecycleAction::Pause)
                    .unwrap();
                Ok(output(None, true, &marker(Some(1))))
            }
            Scripted::Publish(candidate) => {
                PreparedBodyProcess {
                    candidate,
                    overflow_stdout: false,
                }
                .execute(request, control)
                .await
            }
        }
    }
}

struct Fixture {
    _temp: tempfile::TempDir,
    store: WorkflowStore,
    run_id: String,
    generation: u64,
    process: Arc<ScriptedProcess>,
    executor: FixedHostCommandExecutor,
    context: HostCommandResolutionContext,
}

fn fixture(outcomes: Vec<Scripted>) -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let mut context = context(temp.path());
    let store = WorkflowStore::project(&context.project_root);
    let mut run = store
        .create_run(archon_workflow::WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
            name: "host-operational".into(),
            task: "test".into(),
            target_repository_root: None,
            max_parallelism: 1,
            max_agents: 1,
            stages: Vec::new(),
            permissions: Default::default(),
            learning_hooks: Vec::new(),
        })
        .unwrap();
    run.status = RunStatus::Running;
    let mut stage = archon_workflow::run::StageState::pending("host-call");
    stage.status = StageStatus::Running;
    run.stages.insert("host-call".into(), stage);
    store.save_state(&run).unwrap();
    let run_root = store.run_dir(&run.id);
    context.run_staging_root = run_root.join("host-command-staging");
    let process = Arc::new(ScriptedProcess {
        calls: AtomicUsize::new(0),
        spill_dirs: Mutex::new(Vec::new()),
        outcomes,
        run: (store.clone(), run.id.clone()),
    });
    let executor = FixedHostCommandExecutor::with_process(
        fixed_decomposition_catalog("rev-1").unwrap(),
        context.clone(),
        run_root,
        process.clone(),
    );
    Fixture {
        _temp: temp,
        store,
        run_id: run.id,
        generation: run.generation,
        process,
        executor,
        context,
    }
}

fn events(fixture: &Fixture) -> Vec<serde_json::Value> {
    std::fs::read_to_string(fixture.store.events_path(&fixture.run_id))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn events_named<'a>(events: &'a [serde_json::Value], name: &str) -> Vec<&'a serde_json::Value> {
    events
        .iter()
        .filter(|event| event["detail"]["event"] == name)
        .collect()
}

fn lint() -> HostCommandRequest {
    HostCommandRequest::new("task-set-lint", None).unwrap()
}

#[tokio::test(start_paused = true)]
async fn host_command_timeouts_without_progress_recover_to_one_committed_outcome() {
    assert_host_recovery(vec![Scripted::TimedOut(None), Scripted::TimedOut(None)], 2).await;
}

#[tokio::test(start_paused = true)]
async fn host_command_growing_then_unchanged_progress_recovers_without_pause() {
    assert_host_recovery(
        vec![
            Scripted::TimedOut(Some(1)),
            Scripted::TimedOut(Some(2)),
            Scripted::TimedOut(Some(3)),
            Scripted::TimedOut(Some(4)),
            Scripted::TimedOut(Some(4)),
        ],
        5,
    )
    .await;
}

#[tokio::test(start_paused = true)]
async fn host_command_unchanged_progress_recovers_without_operator_action() {
    assert_host_recovery(
        vec![Scripted::TimedOut(Some(4)), Scripted::TimedOut(Some(4))],
        2,
    )
    .await;
}

async fn assert_host_recovery(mut outcomes: Vec<Scripted>, failures: usize) {
    outcomes.push(Scripted::Publish(CANDIDATE.as_bytes().to_vec()));
    let fixture = fixture(outcomes);
    let task_file = fixture.context.task_root.join("TASK-X-010.md");
    seed_frozen_chain(&fixture.context, &task_file);
    let result = fixture
        .executor
        .execute(
            HostCommandRequest::new("land-task-body", Some(CANDIDATE.into())).unwrap(),
            Some(fixture.generation),
        )
        .await
        .unwrap();
    assert!(result.reusable(), "{result:?}");
    assert_eq!(fixture.process.calls.load(Ordering::SeqCst), failures + 1);
    assert_eq!(std::fs::read(&task_file).unwrap(), CANDIDATE.as_bytes());
    let state = fixture.store.load_state(&fixture.run_id).unwrap();
    assert_eq!(state.status, RunStatus::Running);
    assert_eq!(state.generation, fixture.generation);
    assert_eq!(state.stages["host-call"].status, StageStatus::Running);
    assert!(events_named(&events(&fixture), "host_command_operational_pause").is_empty());
    let spills = fixture.process.spill_dirs.lock().unwrap();
    assert_eq!(
        spills
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        failures + 1
    );
}

const CANDIDATE: &str = "# Candidate\n\n```yaml\ntask_id: TASK-X-010\ntitle: Candidate\ncomplexity: low\nstatus: ready\ndepends_on: []\nblocks: []\nimplements: []\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n```\n\n## Focused Tests\n- `test -f TASK-X-010.md`\n";

#[tokio::test(start_paused = true)]
async fn host_command_incomplete_resumable_retries_with_backoff_to_publication() {
    let candidate = CANDIDATE.as_bytes().to_vec();
    let fixture = fixture(vec![
        Scripted::Incomplete(2),
        Scripted::Incomplete(5),
        Scripted::Publish(candidate.clone()),
    ]);
    let task_file = fixture.context.task_root.join("TASK-X-010.md");
    std::fs::write(&task_file, b"live-before").unwrap();
    seed_frozen_chain(&fixture.context, &task_file);
    let request = HostCommandRequest::new("land-task-body", Some(CANDIDATE.into())).unwrap();

    let started = tokio::time::Instant::now();
    let result = fixture
        .executor
        .execute(request, Some(fixture.generation))
        .await
        .unwrap();

    assert!(started.elapsed() >= std::time::Duration::from_millis(1500));
    assert_eq!(fixture.process.calls.load(Ordering::SeqCst), 3);
    assert!(result.reusable(), "{result:?}");
    assert_eq!(std::fs::read(&task_file).unwrap(), candidate);
    let run = fixture.store.load_state(&fixture.run_id).unwrap();
    assert_eq!(run.status, RunStatus::Running);
    assert_eq!(run.generation, fixture.generation);
    assert!(events_named(&events(&fixture), "host_command_operational_pause").is_empty());
}

#[tokio::test]
async fn host_command_permanent_exit_and_stage_refusal_remain_single_attempt() {
    let fixture = fixture(vec![Scripted::Exit(1)]);
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        fixture.executor.execute(lint(), Some(fixture.generation)),
    )
    .await
    .expect("permanent exit must finish")
    .unwrap();
    assert_eq!(result.exit_code, Some(1));
    assert!(!result.reusable() && !result.timed_out);
    assert_eq!(fixture.process.calls.load(Ordering::SeqCst), 1);

    let fixture_b = self::fixture(vec![Scripted::Error(
        "host command 'task-set-lint' failed: permission denied",
    )]);
    let result_b = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        fixture_b
            .executor
            .execute(lint(), Some(fixture_b.generation)),
    )
    .await
    .expect("permanent refusal must finish")
    .unwrap_err();
    let error = result_b;
    assert!(matches!(error, WorkflowError::StageFailed(_)), "{error:?}");
    assert!(error.to_string().contains("permission denied"), "{error:?}");
    assert_eq!(fixture_b.process.calls.load(Ordering::SeqCst), 1);

    for fixture in [&fixture, &fixture_b] {
        let run = fixture.store.load_state(&fixture.run_id).unwrap();
        assert_eq!(run.status, RunStatus::Running);
        assert_eq!(run.generation, fixture.generation);
        assert!(events(fixture).is_empty());
    }
}

#[tokio::test]
async fn an_operator_pause_between_attempts_wins_over_the_retry() {
    let fixture = fixture(vec![Scripted::OperatorPauseThenTimeOut, Scripted::Exit(0)]);
    let error = fixture
        .executor
        .execute(lint(), Some(fixture.generation))
        .await
        .unwrap_err();

    assert!(matches!(error, WorkflowError::ControlPaused(_)), "{error}");
    assert_eq!(fixture.process.calls.load(Ordering::SeqCst), 1);
    let run = fixture.store.load_state(&fixture.run_id).unwrap();
    assert_eq!(run.status, RunStatus::Paused);
    assert_eq!(run.generation, fixture.generation + 1);
    assert!(events_named(&events(&fixture), "host_command_operational_pause").is_empty());
}

#[tokio::test(start_paused = true)]
async fn host_command_zero_progress_timeouts_recover_without_operator_action() {
    assert_host_recovery(
        vec![Scripted::TimedOut(Some(0)), Scripted::TimedOut(Some(0))],
        2,
    )
    .await;
}

#[cfg(unix)]
#[tokio::test]
async fn a_real_timeout_keeps_the_progress_the_child_reported_before_the_kill() {
    // Use a shell and readiness file so startup scan time does not affect the capture.
    let temp = tempfile::tempdir().unwrap();
    let ready = temp.path().join("ready");
    let script = format!(
        "echo '{}' >&2; : > '{}'; exec /bin/sleep 60",
        progress_line(9),
        ready.display()
    );
    let request = ResolvedHostCommand {
        command_id: "test-fixture".into(),
        program: "/bin/sh".into(),
        args: vec!["-c".into(), script],
        cwd: std::env::temp_dir(),
        environment: Default::default(),
        stdin: None,
        timeout_secs: 10,
        max_stdout_bytes: 1024,
        max_stderr_bytes: 1024,
        declared_write_set: Vec::new(),
        remediation_scopes: Default::default(),
        spill_dir: None,
    };
    let (control, _handle) = HostCommandControl::new();
    let supervised = tokio::spawn(
        super::super::workflow_host_command_supervisor::supervise_process_group(
            request, control, None,
        ),
    );
    let started = std::time::Instant::now();
    while !ready.exists() {
        assert!(
            started.elapsed() < std::time::Duration::from_secs(9),
            "the child never reported readiness before the kill"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let output = supervised.await.unwrap().unwrap();
    assert_eq!(classify(&output), Some(OperationalKind::TimedOut));
    assert_eq!(reported_progress(&output.stderr), Some(9));
}

#[cfg(unix)]
#[path = "workflow_host_command_operational_stall_tests.rs"]
mod stall;

#[path = "workflow_host_command_operational_unsettled_tests.rs"]
mod unsettled_tests;

#[cfg(unix)]
#[path = "workflow_host_command_registration_tests.rs"]
mod registration_tests;

#[cfg(unix)]
#[path = "workflow_host_checkpoint_r3_tests.rs"]
mod checkpoint_r3_tests;

#[path = "workflow_host_command_operational_output_cap_tests.rs"]
mod output_cap_tests;

#[path = "workflow_host_command_operational_evidence_tests.rs"]
mod evidence_tests;
#[path = "workflow_host_command_operational_spill_tests.rs"]
mod spill_tests;
