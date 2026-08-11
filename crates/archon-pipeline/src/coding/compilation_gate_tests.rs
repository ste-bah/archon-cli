use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::coding::gates::CompilationGate;

use super::{ChildReap, CleanupOutcome, CommandSpec, TerminationRequest};

const CHILD_MARKER_ENV: &str = "ARCHON_COMPILATION_GATE_CHILD_MARKER";
const CHILD_RELEASE_ENV: &str = "ARCHON_COMPILATION_GATE_CHILD_RELEASE";
const CHILD_MODE_ENV: &str = "ARCHON_COMPILATION_GATE_CHILD_MODE";
const CHILD_GATE_RESULT_ENV: &str = "ARCHON_COMPILATION_GATE_CHILD_GATE_RESULT";
const CHILD_INNER_MARKER_ENV: &str = "ARCHON_COMPILATION_GATE_CHILD_INNER_MARKER";
const CHILD_DESCENDANT_MARKER_ENV: &str = "ARCHON_COMPILATION_GATE_CHILD_DESCENDANT_MARKER";
const CHILD_DESCENDANT_SURVIVOR_ENV: &str = "ARCHON_COMPILATION_GATE_CHILD_DESCENDANT_SURVIVOR";
const CHILD_STDIN_OUTCOME_ENV: &str = "ARCHON_COMPILATION_GATE_CHILD_STDIN_OUTCOME";

/// How often anything here re-checks a marker file.
///
/// This is a polling interval, not a deadline. Every loop that uses it ends on
/// the event it is watching for, so load changes how many times a loop turns
/// and nothing else.
const POLL_INTERVAL: Duration = Duration::from_millis(5);

/// The gate limit given to the two fixtures that are not about timing out.
///
/// [`compilation_child_receives_eof_before_parent_stdin_closes`] and
/// [`compilation_wait_keeps_current_thread_runtime_responsive`] both have to
/// pass `run_command` *some* limit, and neither asserts anything about how long
/// the child takes: each waits on an event the child itself produces. So the
/// value is sized so that only a wedged child can reach it, which leaves it out
/// of the result on any machine at any load.
///
/// Sixty seconds is twelve times the worst process-creation cost measured on a
/// loaded 32-core Windows box (`.config/nextest.toml` records 4.96s for a
/// single spawn), and comfortably inside nextest's `60s x 4` `terminate-after`
/// ceiling, so a fixture that genuinely deadlocked still fails through the
/// assertions below - with their message - rather than through the harness
/// cancelling the run. It is a hang guard, not a budget: moving it up or down
/// does not change what passes.
const HANG_GUARD: Duration = Duration::from_secs(60);

#[test]
fn controlled_child() {
    let Ok(marker) = std::env::var(CHILD_MARKER_ENV) else {
        return;
    };
    std::fs::write(marker, "started").unwrap();

    match std::env::var(CHILD_MODE_ENV).as_deref() {
        Ok("await-release") => await_release(),
        Ok("read-stdin") => read_stdin(),
        Ok("run-stdin-reader") => run_stdin_reader(),
        Ok("exit-after-spawning-descendant") => spawn_descendant(),
        Ok("descendant-hold-stream") => hold_descendant_stream(),
        _ => loop {
            std::thread::sleep(POLL_INTERVAL);
        },
    }
}

/// Block until the parent says so, with no deadline of its own.
///
/// The deadline this used to carry was the flake: the parent writes `release`
/// only once it has observed this process's startup marker, so a child that
/// gave up on a timer would end the wait before the parent got its observation
/// in, and whether that happened was decided by how busy the machine was. The
/// wait is now ended by the parent's signal and nothing else.
///
/// Nothing is leaked by looping forever: the gate wraps this process in
/// `KillOnDrop` plus a job object (Windows) or process group (unix) and owns
/// its lifetime, which is the same arrangement the default arm of
/// [`controlled_child`] already relies on.
fn await_release() {
    let release = std::env::var(CHILD_RELEASE_ENV).unwrap();
    while !Path::new(&release).exists() {
        std::thread::sleep(POLL_INTERVAL);
    }
}

fn read_stdin() {
    let outcome = std::env::var(CHILD_STDIN_OUTCOME_ENV).unwrap();
    let result = std::io::stdin().read_to_end(&mut Vec::new());
    std::fs::write(outcome, if result.is_ok() { "eof" } else { "read-error" }).unwrap();
}

fn run_stdin_reader() {
    let inner_marker = std::env::var(CHILD_INNER_MARKER_ENV).unwrap();
    let stdin_outcome = std::env::var(CHILD_STDIN_OUTCOME_ENV).unwrap();
    let gate_result = std::env::var(CHILD_GATE_RESULT_ENV).unwrap();
    let project_root = std::env::current_dir().unwrap();
    let spec = controlled_child_spec(&project_root, Path::new(&inner_marker), None)
        .with_env(CHILD_MODE_ENV, "read-stdin")
        .with_env(CHILD_STDIN_OUTCOME_ENV, stdin_outcome);
    let result = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(CompilationGate.run_command(spec, HANG_GUARD));
    std::fs::write(
        gate_result,
        if result.gate_passed {
            "passed"
        } else {
            "failed"
        },
    )
    .unwrap();
}

#[allow(clippy::zombie_processes)] // Fixture intentionally lets descendant outlive direct child.
fn spawn_descendant() {
    let descendant_marker = std::env::var(CHILD_DESCENDANT_MARKER_ENV).unwrap();
    let mut descendant = Command::new(std::env::current_exe().unwrap());
    descendant
        .args([
            "--exact",
            "coding::compilation_gate::compilation_gate_tests::controlled_child",
            "--nocapture",
        ])
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .env(CHILD_MARKER_ENV, &descendant_marker)
        .env(CHILD_MODE_ENV, "descendant-hold-stream");
    if let Ok(survivor) = std::env::var(CHILD_DESCENDANT_SURVIVOR_ENV) {
        descendant.env(CHILD_DESCENDANT_SURVIVOR_ENV, survivor);
    }
    descendant.spawn().unwrap();
    assert!(
        wait_for_file(Path::new(&descendant_marker), Duration::from_secs(1)),
        "descendant must start before direct child exits"
    );
}

fn hold_descendant_stream() {
    let survivor = std::env::var(CHILD_DESCENDANT_SURVIVOR_ENV).ok();
    std::thread::sleep(Duration::from_millis(1200));
    if let Some(survivor) = survivor {
        std::fs::write(survivor, "survived-timeout").unwrap();
    }
    std::thread::sleep(Duration::from_millis(1500));
}

/// The gate must give its child a stdin of its own, not the one it inherited.
///
/// Three processes: this test holds the write end of `runner`'s stdin pipe;
/// `runner` hosts a [`CompilationGate`] on its own runtime; the gate spawns a
/// reader that drains stdin to EOF. If the gate ever handed the reader an
/// inherited stdin, the reader would sit on a pipe only this test can close and
/// the gate would not return until it did.
///
/// What proves that did not happen is `runner`'s exit. `runner` never reads its
/// own stdin, so it exits as soon as the gate it hosts returns - and it exits
/// here while `runner_stdin` is still open, which is the ordering the name
/// claims. Waiting on the exit replaces three `wait_for_file` polls that each
/// had a one-second budget; spawning a debug-profile process costs what the
/// machine is doing, so those budgets asked whether the box was busy, not
/// whether the gate rewired stdin.
#[test]
fn compilation_child_receives_eof_before_parent_stdin_closes() {
    let temp = tempfile::tempdir().unwrap();
    let runner_marker = temp.path().join("runner-started");
    let inner_marker = temp.path().join("reader-started");
    let stdin_outcome = temp.path().join("stdin-outcome");
    let gate_result = temp.path().join("gate-result");
    let mut runner = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "coding::compilation_gate::compilation_gate_tests::controlled_child",
            "--nocapture",
        ])
        .current_dir(temp.path())
        .stdin(Stdio::piped())
        .env(CHILD_MARKER_ENV, &runner_marker)
        .env(CHILD_MODE_ENV, "run-stdin-reader")
        .env(CHILD_INNER_MARKER_ENV, &inner_marker)
        .env(CHILD_STDIN_OUTCOME_ENV, &stdin_outcome)
        .env(CHILD_GATE_RESULT_ENV, &gate_result)
        .spawn()
        .unwrap();
    // `Child::wait` closes `stdin` before waiting, precisely to avoid the
    // deadlock this test wants to be exposed to. Taking the handle moves the
    // write end here so it survives the wait below.
    let runner_stdin = runner.stdin.take().unwrap();

    let status = runner.wait().unwrap();
    // Read everything while the pipe is still open, so the evidence is gathered
    // under the condition being asserted rather than after it is relaxed.
    let runner_started = runner_marker.exists();
    let reader_started = inner_marker.exists();
    let gate_outcome = std::fs::read_to_string(&gate_result).unwrap_or_default();
    let stdin_outcome = std::fs::read_to_string(&stdin_outcome).unwrap_or_default();
    drop(runner_stdin);

    assert!(runner_started, "controlled gate runner must start");
    assert!(reader_started, "controlled stdin reader must start");
    assert!(status.success(), "controlled gate runner must exit cleanly");
    assert_eq!(
        gate_outcome, "passed",
        "deadlock guard: gate must finish while its parent's stdin stays open"
    );
    assert_eq!(stdin_outcome, "eof");
}

#[test]
fn cleanup_outcome_formats_only_observable_states() {
    assert_eq!(
        CleanupOutcome::AlreadyExited.evidence(),
        "direct child already exited and was reaped"
    );
    assert_eq!(
        CleanupOutcome::TerminationRequestAccepted {
            reap: ChildReap::Succeeded,
        }
        .evidence(),
        "direct child termination request accepted; direct child reaped"
    );
    assert_eq!(
        CleanupOutcome::TerminationRequestFailed {
            reap: ChildReap::Succeeded,
        }
        .evidence(),
        "direct child termination request failed; direct child reaped"
    );
    assert_eq!(
        CleanupOutcome::TerminationRequestAccepted {
            reap: ChildReap::Failed,
        }
        .evidence(),
        "direct child termination request accepted; direct child reap failed"
    );
    assert_eq!(
        CleanupOutcome::InspectionFailed {
            termination: TerminationRequest::Failed,
            reap: ChildReap::Succeeded,
        }
        .evidence(),
        "direct child status inspection failed; direct child termination request failed; direct child reaped"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn compilation_timeout_terminates_descendant_when_direct_child_exited() {
    let temp = tempfile::tempdir().unwrap();
    let child_marker = temp.path().join("child-started");
    let descendant_marker = temp.path().join("descendant-started");
    let spec = controlled_child_spec(temp.path(), &child_marker, None)
        .with_env(CHILD_MODE_ENV, "exit-after-spawning-descendant")
        .with_env(
            CHILD_DESCENDANT_MARKER_ENV,
            descendant_marker.display().to_string(),
        );

    let result = CompilationGate
        .run_command(spec, Duration::from_secs(1))
        .await;

    assert!(child_marker.exists(), "direct child must have started");
    assert!(
        wait_for_file(&descendant_marker, Duration::from_secs(1)),
        "descendant must have inherited a pipe before the timeout"
    );
    assert!(!result.gate_passed);
    assert_eq!(result.failures[0].description, "Compilation timed out");
    assert!(
        result
            .evidence
            .contains("direct child already exited and was reaped")
    );
}

#[tokio::test(flavor = "current_thread")]
async fn compilation_timeout_terminates_descendant_processes() {
    let temp = tempfile::tempdir().unwrap();
    let child_marker = temp.path().join("child-started");
    let descendant_marker = temp.path().join("descendant-started");
    let survivor_marker = temp.path().join("descendant-survived");
    let spec = controlled_child_spec(temp.path(), &child_marker, None)
        .with_env(CHILD_MODE_ENV, "exit-after-spawning-descendant")
        .with_env(CHILD_DESCENDANT_MARKER_ENV, &descendant_marker)
        .with_env(CHILD_DESCENDANT_SURVIVOR_ENV, &survivor_marker);

    let result = CompilationGate
        .run_command(spec, Duration::from_millis(500))
        .await;
    std::thread::sleep(Duration::from_millis(900));

    assert!(!result.gate_passed);
    assert!(descendant_marker.exists(), "descendant must have started");
    assert!(
        !survivor_marker.exists(),
        "timed-out compilation descendant survived process-tree cleanup"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn compilation_timeout_requests_termination_and_reaps_child_with_stable_evidence() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("child-started");
    let spec = controlled_child_spec(temp.path(), &marker, None);
    let gate = CompilationGate;

    let result = gate.run_command(spec, Duration::from_millis(100)).await;

    assert!(marker.exists(), "controlled child must have started");
    assert!(!result.gate_passed);
    assert_eq!(result.failures[0].description, "Compilation timed out");
    assert!(
        result
            .evidence
            .contains("direct child termination request accepted; direct child reaped")
    );
    assert_eq!(result.failures[0].details, result.evidence);
}

#[tokio::test]
async fn compilation_spawn_failure_returns_failed_gate_record() {
    let temp = tempfile::tempdir().unwrap();
    let gate = CompilationGate;
    let result = gate
        .run_command(
            CommandSpec::new("archon-missing-compilation-command", [], temp.path()),
            Duration::from_secs(1),
        )
        .await;

    assert!(!result.gate_passed);
    assert_eq!(
        result.failures[0].description,
        "Build command failed to execute"
    );
    assert!(
        result
            .evidence
            .starts_with("Failed to execute archon-missing-compilation-command:")
    );
}

/// Awaiting the compilation child must leave the current-thread runtime free to
/// run everything else on it.
///
/// The property is observed rather than timed. The child blocks until `release`
/// exists, and the only thing that can create `release` is this test body -
/// another task on the same single-threaded runtime as `gate_task`. So the gate
/// completing at all is the evidence: if `run_command` held the thread while it
/// waited, this task would never be polled again, `release` would never be
/// written, and the gate could not return.
///
/// That is also what the old shape could not do. It waited on a marker file
/// from an OS thread and awaited a `oneshot` under a two-second timeout, but a
/// blocked current-thread runtime cannot fire its own timer either, so a real
/// violation hung there just as it does here - the budgets only ever separated
/// a busy machine from a quiet one. Worse, the child gave up on `release` after
/// one second of its own, so a slow spawn let the gate finish before the
/// `is_finished` check ran and turned the claim inside out. Both budgets are
/// gone; the check is now true by construction.
#[tokio::test(flavor = "current_thread")]
async fn compilation_wait_keeps_current_thread_runtime_responsive() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("child-started");
    let release = temp.path().join("release-child");
    let spec = controlled_child_spec(temp.path(), &marker, Some(&release));
    let gate = CompilationGate;

    let gate_task = tokio::spawn(async move { gate.run_command(spec, HANG_GUARD).await });

    // Reaching the far side of this loop is itself the observation: it can only
    // happen if the runtime kept scheduling this task while `gate_task` sat on
    // its child. The loop ends on the child's own startup signal, never on a
    // clock.
    while !marker.exists() {
        tokio::time::sleep(POLL_INTERVAL).await;
    }
    assert!(
        !gate_task.is_finished(),
        "the async runtime must progress while the compilation command waits"
    );
    std::fs::write(&release, "release").unwrap();

    let result = gate_task.await.unwrap();
    assert!(
        result.gate_passed,
        "controlled child should exit successfully"
    );
}

fn wait_for_file(path: &Path, timeout: Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while !path.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(POLL_INTERVAL);
    }
    path.exists()
}

fn controlled_child_spec(
    project_root: &Path,
    marker: &Path,
    release: Option<&Path>,
) -> CommandSpec {
    let mut spec = CommandSpec::new(
        std::env::current_exe().unwrap(),
        [
            "--exact".to_owned(),
            "coding::compilation_gate::compilation_gate_tests::controlled_child".to_owned(),
            "--nocapture".to_owned(),
        ],
        project_root,
    )
    .with_env(CHILD_MARKER_ENV, marker.display().to_string());

    if let Some(release) = release {
        spec = spec
            .with_env(CHILD_MODE_ENV, "await-release")
            .with_env(CHILD_RELEASE_ENV, release.display().to_string());
    }

    spec
}
