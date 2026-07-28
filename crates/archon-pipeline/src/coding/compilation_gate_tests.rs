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
const CHILD_STDIN_OUTCOME_ENV: &str = "ARCHON_COMPILATION_GATE_CHILD_STDIN_OUTCOME";

#[test]
fn controlled_child() {
    let Ok(marker) = std::env::var(CHILD_MARKER_ENV) else {
        return;
    };
    std::fs::write(marker, "started").unwrap();

    match std::env::var(CHILD_MODE_ENV).as_deref() {
        Ok("await-release") => {
            let release = std::env::var(CHILD_RELEASE_ENV).unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(1);
            while !Path::new(&release).exists() && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        Ok("read-stdin") => {
            let outcome = std::env::var(CHILD_STDIN_OUTCOME_ENV).unwrap();
            let result = std::io::stdin().read_to_end(&mut Vec::new());
            std::fs::write(outcome, if result.is_ok() { "eof" } else { "read-error" }).unwrap();
        }
        Ok("run-stdin-reader") => {
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
                .block_on(CompilationGate.run_command(spec, Duration::from_millis(100)));
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
        Ok("exit-after-spawning-descendant") => {
            let descendant_marker = std::env::var(CHILD_DESCENDANT_MARKER_ENV).unwrap();
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "coding::compilation_gate::compilation_gate_tests::controlled_child",
                    "--nocapture",
                ])
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .env(CHILD_MARKER_ENV, descendant_marker)
                .env(CHILD_MODE_ENV, "descendant-hold-stream")
                .spawn()
                .unwrap();
        }
        Ok("descendant-hold-stream") => {
            let deadline = std::time::Instant::now() + Duration::from_millis(300);
            while std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        _ => loop {
            std::thread::sleep(Duration::from_millis(5));
        },
    }
}

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
    let runner_stdin = runner.stdin.take().unwrap();

    assert!(
        wait_for_file(&runner_marker, Duration::from_secs(1)),
        "controlled gate runner must start"
    );
    assert!(
        wait_for_file(&inner_marker, Duration::from_secs(1)),
        "controlled stdin reader must start"
    );
    assert!(
        wait_for_file(&gate_result, Duration::from_secs(1)),
        "deadlock guard: gate must finish while its parent's stdin stays open"
    );
    let gate_outcome = std::fs::read_to_string(&gate_result).unwrap();
    let stdin_outcome = std::fs::read_to_string(&stdin_outcome).unwrap_or_default();

    drop(runner_stdin);
    assert!(runner.wait().unwrap().success());
    assert_eq!(gate_outcome, "passed");
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
async fn compilation_timeout_reports_already_exited_direct_child_when_descendant_holds_pipe() {
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
        .run_command(spec, Duration::from_millis(100))
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
    assert!(!result.evidence.contains("killed"));
    assert!(!result.evidence.contains("termination request accepted"));
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

#[tokio::test(flavor = "current_thread")]
async fn compilation_wait_keeps_current_thread_runtime_responsive() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("child-started");
    let release = temp.path().join("release-child");
    let spec = controlled_child_spec(temp.path(), &marker, Some(&release));
    let gate = CompilationGate;
    let (progress_tx, progress_rx) = tokio::sync::oneshot::channel();

    let gate_task =
        tokio::spawn(async move { gate.run_command(spec, Duration::from_secs(2)).await });
    let marker_for_thread = marker.clone();
    std::thread::spawn(move || {
        let started = wait_for_file(&marker_for_thread, Duration::from_secs(1));
        let _ = progress_tx.send(started);
    });

    let started = tokio::time::timeout(Duration::from_secs(2), progress_rx)
        .await
        .expect("controlled child startup signal timed out")
        .expect("controlled child startup sender dropped");
    assert!(started, "controlled child did not write its startup marker");
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
        std::thread::sleep(Duration::from_millis(5));
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
