//! Issue #255: an operational ending of a host command is retried while it
//! makes progress and then pauses the run; it never fails the work.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

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
    OperationalAttempt {
        attempt: n,
        reason: "timed_out",
        elapsed_secs: 1,
        progress,
    }
}

fn output(exit_code: Option<i32>, timed_out: bool, stderr: &str) -> SupervisedProcessOutput {
    SupervisedProcessOutput {
        exit_code,
        timed_out,
        stdout: Vec::new(),
        stderr: stderr.as_bytes().to_vec(),
        stdout_bytes: 0,
        stderr_bytes: stderr.len() as u64,
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
    // No marker: exactly one retry.
    assert_eq!(next_step(&[attempt(1, None)]), Retry);
    assert_eq!(
        next_step(&[attempt(1, None), attempt(2, None)]),
        Pause("no_progress_evidence")
    );
    // Growing progress: retried with no total work or time budget.
    assert_eq!(next_step(&[attempt(1, Some(1))]), Retry);
    let growing = |n: u32| -> Vec<OperationalAttempt> {
        (1..=n).map(|i| attempt(i, Some(u64::from(i)))).collect()
    };
    assert_eq!(next_step(&growing(10)), Retry);
    assert_eq!(next_step(&growing(130)), Retry);
    // Progress that does not grow, or a marker that disappears.
    // No baseline yet: a first marker, even 0, gets the no-marker retry.
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

/// One scripted process outcome per attempt; the last repeats.
struct ScriptedProcess {
    calls: AtomicUsize,
    outcomes: Vec<Scripted>,
    run: (WorkflowStore, String),
}

#[derive(Clone)]
enum Scripted {
    TimedOut(Option<u64>),
    Incomplete(u64),
    Exit(i32),
    Error(&'static str),
    Operational(&'static str),
    Publish(Vec<u8>),
    /// The operator pauses while the attempt runs; it then times out.
    OperatorPauseThenTimeOut,
    /// Exits with this status and stderr (Issue 338).
    Stderr(i32, String),
}

#[async_trait::async_trait]
impl HostCommandProcessAdapter for ScriptedProcess {
    async fn execute(
        &self,
        request: ResolvedHostCommand,
        control: HostCommandControl,
    ) -> WorkflowResult<SupervisedProcessOutput> {
        let index = self.calls.fetch_add(1, Ordering::SeqCst);
        let scripted = self.outcomes[index.min(self.outcomes.len() - 1)].clone();
        // A killed attempt leaves partial staging behind; the next attempt
        // must not see it, or the audit refuses the extra file.
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
                Ok(output(Some(EXIT_INCOMPLETE_RESUMABLE), false, &stderr))
            }
            Scripted::Exit(code) => Ok(output(Some(code), false, "genuine failure")),
            Scripted::Stderr(code, stderr) => Ok(output(Some(code), false, &stderr)),
            Scripted::Error(text) => Err(WorkflowError::StageFailed(text.into())),
            Scripted::Operational(text) => Err(WorkflowError::HostOperational(text.into())),
            Scripted::OperatorPauseThenTimeOut => {
                archon_workflow::LifecycleController::new(self.run.0.clone())
                    .apply(&self.run.1, archon_workflow::LifecycleAction::Pause)
                    .unwrap();
                Ok(output(None, true, &marker(Some(1))))
            }
            Scripted::Publish(candidate) => {
                PreparedBodyProcess { candidate }
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
    // The script host marks the call's stage running before dispatch.
    run.status = RunStatus::Running;
    let mut stage = archon_workflow::run::StageState::pending("host-call");
    stage.status = StageStatus::Running;
    run.stages.insert("host-call".into(), stage);
    store.save_state(&run).unwrap();
    let run_root = store.run_dir(&run.id);
    context.run_staging_root = run_root.join("host-command-staging");
    let process = Arc::new(ScriptedProcess {
        calls: AtomicUsize::new(0),
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

#[tokio::test]
async fn a_timed_out_call_is_retried_once_then_pauses_the_run() {
    let fixture = fixture(vec![Scripted::TimedOut(None)]);
    let error = fixture
        .executor
        .execute(lint(), Some(fixture.generation))
        .await
        .unwrap_err();

    assert_eq!(fixture.process.calls.load(Ordering::SeqCst), 2);
    let WorkflowError::ControlPaused(message) = &error else {
        panic!("an operational limit pauses, never fails: {error:?}");
    };
    assert!(message.contains("'task-set-lint'"), "{message}");
    assert!(message.contains("timed_out"), "{message}");
    assert!(
        message.contains(&format!(
            "archon workflow resume --live --yes {}",
            fixture.run_id
        )),
        "{message}"
    );
    let run = fixture.store.load_state(&fixture.run_id).unwrap();
    assert_eq!(run.status, RunStatus::Paused);
    assert_eq!(run.generation, fixture.generation + 1);
    assert_eq!(run.stages["host-call"].status, StageStatus::Paused);

    let events = events(&fixture);
    let retries = events_named(&events, "host_command_operational_retry");
    assert_eq!(retries.len(), 1, "{events:?}");
    assert_eq!(retries[0]["kind"], "stage_stalled");
    assert_eq!(retries[0]["detail"]["attempt"], 1);
    assert_eq!(retries[0]["detail"]["reason"], "timed_out");
    assert!(retries[0]["detail"]["elapsed_secs"].is_u64());
    assert!(retries[0]["detail"]["progress"].is_null());
    let pauses = events_named(&events, "host_command_operational_pause");
    assert_eq!(pauses.len(), 1, "{events:?}");
    let pause = &pauses[0]["detail"];
    assert_eq!(pauses[0]["kind"], "paused");
    assert_eq!(pause["cause"], "no_progress_evidence");
    assert_eq!(pause["command_id"], "task-set-lint");
    assert_eq!(pause["attempts"].as_array().unwrap().len(), 2);
    assert!(pause["limit_secs"].as_u64().unwrap() > 0);
    assert_eq!(pause["generation"], fixture.generation + 1);
}

#[tokio::test]
async fn growing_progress_keeps_retrying_until_an_attempt_adds_nothing() {
    let fixture = fixture(vec![
        Scripted::TimedOut(Some(1)),
        Scripted::TimedOut(Some(2)),
        Scripted::TimedOut(Some(3)),
        Scripted::TimedOut(Some(4)),
        Scripted::TimedOut(Some(4)),
    ]);
    let error = fixture
        .executor
        .execute(lint(), Some(fixture.generation))
        .await
        .unwrap_err();

    assert!(matches!(error, WorkflowError::ControlPaused(_)), "{error}");
    assert_eq!(fixture.process.calls.load(Ordering::SeqCst), 5);
    let events = events(&fixture);
    let progress: Vec<_> = events_named(&events, "host_command_operational_retry")
        .iter()
        .map(|event| event["detail"]["progress"].as_u64().unwrap())
        .collect();
    assert_eq!(progress, vec![1, 2, 3, 4]);
    let pauses = events_named(&events, "host_command_operational_pause");
    assert_eq!(pauses[0]["detail"]["cause"], "no_progress");
}

#[tokio::test]
async fn progress_that_stops_growing_pauses_without_a_further_retry() {
    let fixture = fixture(vec![Scripted::TimedOut(Some(4))]);
    let error = fixture
        .executor
        .execute(lint(), Some(fixture.generation))
        .await
        .unwrap_err();

    assert!(matches!(error, WorkflowError::ControlPaused(_)), "{error}");
    assert_eq!(fixture.process.calls.load(Ordering::SeqCst), 2);
    let events = events(&fixture);
    let pauses = events_named(&events, "host_command_operational_pause");
    assert_eq!(pauses[0]["detail"]["cause"], "no_progress");
}

const CANDIDATE: &str = "# Candidate\n\n```yaml\ntask_id: TASK-X-010\ntitle: Candidate\ncomplexity: low\nstatus: ready\ndepends_on: []\nblocks: []\nimplements: []\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n```\n\n## Focused Tests\n- `test -f TASK-X-010.md`\n";

#[tokio::test]
async fn an_incomplete_resumable_call_with_growing_progress_continues_to_publication() {
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

    let result = fixture
        .executor
        .execute(request, Some(fixture.generation))
        .await
        .unwrap();

    assert_eq!(fixture.process.calls.load(Ordering::SeqCst), 3);
    assert!(result.reusable(), "{result:?}");
    assert_eq!(std::fs::read(&task_file).unwrap(), candidate);
    let run = fixture.store.load_state(&fixture.run_id).unwrap();
    assert_eq!(run.status, RunStatus::Running);
    assert_eq!(run.generation, fixture.generation);
    let events = events(&fixture);
    let retries = events_named(&events, "host_command_operational_retry");
    let reasons: Vec<_> = retries
        .iter()
        .map(|event| event["detail"]["reason"].as_str().unwrap())
        .collect();
    assert_eq!(reasons, vec!["incomplete_resumable"; 2]);
    assert!(events_named(&events, "host_command_operational_pause").is_empty());
}

#[tokio::test]
async fn genuine_failures_still_fail_once_without_retry_or_pause() {
    let fixture = fixture(vec![Scripted::Exit(1)]);
    let result = fixture
        .executor
        .execute(lint(), Some(fixture.generation))
        .await
        .unwrap();
    assert_eq!(result.exit_code, Some(1));
    assert!(!result.reusable() && !result.timed_out);
    assert_eq!(fixture.process.calls.load(Ordering::SeqCst), 1);

    let fixture_b = self::fixture(vec![Scripted::Error(
        "host command 'task-set-lint' failed: permission denied",
    )]);
    let error = fixture_b
        .executor
        .execute(lint(), Some(fixture_b.generation))
        .await
        .unwrap_err();
    assert!(matches!(error, WorkflowError::StageFailed(_)), "{error}");
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
    // Only the operator's transition: the executor did not pause it again.
    assert_eq!(run.generation, fixture.generation + 1);
    assert!(events_named(&events(&fixture), "host_command_operational_pause").is_empty());
}

#[tokio::test]
async fn a_first_zero_progress_marker_gets_the_same_single_retry_as_no_marker() {
    let fixture = fixture(vec![Scripted::TimedOut(Some(0))]);
    let error = fixture
        .executor
        .execute(lint(), Some(fixture.generation))
        .await
        .unwrap_err();

    assert!(matches!(error, WorkflowError::ControlPaused(_)), "{error}");
    assert_eq!(fixture.process.calls.load(Ordering::SeqCst), 2);
    let events = events(&fixture);
    let pauses = events_named(&events, "host_command_operational_pause");
    assert_eq!(pauses[0]["detail"]["cause"], "no_progress");
}

#[cfg(unix)]
#[tokio::test]
async fn a_real_timeout_keeps_the_progress_the_child_reported_before_the_kill() {
    // `/bin/sh -c`, not a freshly written script: a new executable can wait
    // seconds on the OS scan before it runs. The child writes the marker,
    // then a readiness file; the test asserts readiness came well before the
    // kill, so the capture never depends on timing luck.
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
