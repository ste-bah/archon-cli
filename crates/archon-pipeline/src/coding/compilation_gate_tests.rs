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
const CHILD_DESCENDANT_PROBE_ENV: &str = "ARCHON_COMPILATION_GATE_CHILD_DESCENDANT_PROBE";
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
    // The marker carries this process's id, not just its existence, so a parent
    // can wait for the *process* rather than for a file. See [`read_pid`].
    std::fs::write(marker, std::process::id().to_string()).unwrap();

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
    for key in [CHILD_DESCENDANT_PROBE_ENV, CHILD_DESCENDANT_SURVIVOR_ENV] {
        if let Ok(value) = std::env::var(key) {
            descendant.env(key, value);
        }
    }
    descendant.spawn().unwrap();

    // Block until the descendant is up. This is a precondition of exiting, not
    // a claim - what the gate does with an already-exited direct child is only
    // interesting once a descendant exists to hold the output pipe open - so it
    // waits on the event instead of asserting a deadline.
    while !Path::new(&descendant_marker).exists() {
        std::thread::sleep(POLL_INTERVAL);
    }
}

fn hold_descendant_stream() {
    // Hold the inherited output pipe open. Process-tree cleanup is supposed to
    // end this process right here. The probe is the parent's release: it is
    // written only after cleanup has returned, so a descendant that reaches the
    // survivor write announces that it outlived cleanup - and one that was
    // killed cannot reach it, because a dead process cannot see a file created
    // after its death. The fixture used to sleep 1200ms and write the survivor
    // unconditionally, which meant a slow box moved the write past the window
    // the parent was watching and the check passed having observed nothing.
    let probe = std::env::var(CHILD_DESCENDANT_PROBE_ENV).unwrap();
    while !Path::new(&probe).exists() {
        std::thread::sleep(POLL_INTERVAL);
    }
    if let Ok(survivor) = std::env::var(CHILD_DESCENDANT_SURVIVOR_ENV) {
        std::fs::write(survivor, "survived-timeout").unwrap();
    }
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

/// A descendant still holding the output pipe keeps the gate waiting even after
/// the direct child has gone, and the gate must say so: it reaps the direct
/// child and reports `AlreadyExited` rather than claiming it killed something.
///
/// The precondition - direct child exited, descendant alive - used to be
/// asserted against a one-second budget after a one-second gate limit, which is
/// really the question "can this box start two debug-profile processes that
/// fast". It is now established before the deadline exists at all: see
/// [`hold_until_exited`] for how the state is reached and [`fire_deadline`] for
/// how the limit is then brought on.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn compilation_timeout_terminates_descendant_when_direct_child_exited() {
    let temp = tempfile::tempdir().unwrap();
    let child_marker = temp.path().join("child-started");
    let descendant_marker = temp.path().join("descendant-started");
    let probe = temp.path().join("descendant-release");
    let spec = controlled_child_spec(temp.path(), &child_marker, None)
        .with_env(CHILD_MODE_ENV, "exit-after-spawning-descendant")
        .with_env(CHILD_DESCENDANT_MARKER_ENV, &descendant_marker)
        .with_env(CHILD_DESCENDANT_PROBE_ENV, &probe);

    const LIMIT: Duration = Duration::from_secs(1);
    let gate_task = tokio::spawn(async move { CompilationGate.run_command(spec, LIMIT).await });
    // Establish the exact state the gate is asked to describe: the descendant
    // running and holding the output pipe, the direct child gone. The direct
    // child does not exit until its descendant is up, so waiting for it to exit
    // subsumes the descendant's own start.
    let direct_child = hold_until_started(&child_marker).await;
    hold_until_started(&descendant_marker).await;
    hold_until_exited(direct_child).await;
    fire_deadline(LIMIT).await;
    let result = gate_task.await.unwrap();
    // Release any descendant that outlived cleanup, so a regression here leaves
    // nothing running behind it.
    std::fs::write(&probe, "release").unwrap();

    assert!(child_marker.exists(), "direct child must have started");
    assert!(
        descendant_marker.exists(),
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

/// A timed-out compilation must take its descendants with it.
///
/// Two changes make this load-proof. The deadline is now virtual and cannot
/// arrive before the descendant is up, so "descendant must have started" is
/// established rather than raced for. And the survivor check no longer leans on
/// a fixed observation window that a slow box quietly turned vacuous - the
/// descendant is released only after cleanup has returned, and the test then
/// waits for it to leave the process table. Waiting for the process is the
/// claim; the survivor file is what makes a surviving descendant say so
/// immediately instead of the wait spinning in silence.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn compilation_timeout_terminates_descendant_processes() {
    let temp = tempfile::tempdir().unwrap();
    let child_marker = temp.path().join("child-started");
    let descendant_marker = temp.path().join("descendant-started");
    let probe = temp.path().join("descendant-release");
    let survivor_marker = temp.path().join("descendant-survived");
    let spec = controlled_child_spec(temp.path(), &child_marker, None)
        .with_env(CHILD_MODE_ENV, "exit-after-spawning-descendant")
        .with_env(CHILD_DESCENDANT_MARKER_ENV, &descendant_marker)
        .with_env(CHILD_DESCENDANT_PROBE_ENV, &probe)
        .with_env(CHILD_DESCENDANT_SURVIVOR_ENV, &survivor_marker);

    const LIMIT: Duration = Duration::from_millis(500);
    let gate_task = tokio::spawn(async move { CompilationGate.run_command(spec, LIMIT).await });
    let direct_child = hold_until_started(&child_marker).await;
    let descendant = hold_until_started(&descendant_marker).await;
    hold_until_exited(direct_child).await;
    fire_deadline(LIMIT).await;
    let result = gate_task.await.unwrap();

    assert!(!result.gate_passed);
    assert!(descendant_marker.exists(), "descendant must have started");
    std::fs::write(&probe, "release").unwrap();
    await_process_exit(descendant, &survivor_marker);
    assert!(
        !survivor_marker.exists(),
        "timed-out compilation descendant survived process-tree cleanup"
    );
}

/// When the limit expires with the direct child still running, the gate must
/// ask it to terminate, reap it, and describe exactly that - with the evidence
/// mirrored into the failure detail.
///
/// The child running at that moment is the precondition, and asserting it after
/// a hundred-millisecond real deadline was asking whether the box could start a
/// debug binary in a hundred milliseconds. The limit is now virtual and is
/// brought on by [`fire_deadline`] only once the child has reached its own code,
/// so the cleanup path is always exercised against a child that is genuinely
/// alive.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn compilation_timeout_requests_termination_and_reaps_child_with_stable_evidence() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("child-started");
    let spec = controlled_child_spec(temp.path(), &marker, None);
    let gate = CompilationGate;

    const LIMIT: Duration = Duration::from_millis(100);
    let gate_task = tokio::spawn(async move { gate.run_command(spec, LIMIT).await });
    hold_until_started(&marker).await;
    fire_deadline(LIMIT).await;
    let result = gate_task.await.unwrap();

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

// Split out to keep this file inside the 500-line limit; the tests above and
// the fixtures they drive belong together, the waiting primitives do not.
#[path = "compilation_gate_tests/support.rs"]
mod support;

use support::{
    await_process_exit, controlled_child_spec, fire_deadline, hold_until_exited, hold_until_started,
};
