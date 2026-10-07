use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use archon_workflow::RemediationScope;

use super::workflow_host_command_catalog::ResolvedHostCommand;
use super::workflow_host_command_supervisor::{
    HostCommandControl, HostCommandSignal, supervise_process_group,
};

/// Writes a fixture script. It is run as an argument of the system shell, never
/// executed itself: on macOS every first exec of a newly written file waits for
/// a Gatekeeper scan, which queues behind any other assessment on the machine
/// (an online notarization lookup takes seconds) and so used to consume the
/// whole supervision timeout before the fixture ran a single line.
fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, format!("set -eu\n{body}\n")).unwrap();
    path
}

fn command(script: PathBuf) -> ResolvedHostCommand {
    ResolvedHostCommand {
        command_id: "test-fixture".into(),
        program: PathBuf::from("/bin/sh"),
        args: vec![script.to_string_lossy().into_owned()],
        cwd: std::env::temp_dir(),
        // A resolved host request carries process essentials even under None.
        environment: ["PATH", "HOME"]
            .into_iter()
            .filter_map(|name| std::env::var_os(name).map(|value| (name.to_string(), value)))
            .collect(),
        stdin: None,
        timeout_secs: 5,
        max_stdout_bytes: 1024 * 1024,
        max_stderr_bytes: 1024 * 1024,
        declared_write_set: Vec::new(),
        remediation_scopes: BTreeSet::from([RemediationScope::Operational]),
    }
}

#[tokio::test]
async fn supervisor_waits_for_exit_when_the_child_closes_its_pipes_early() {
    // The drain tasks hold the only supervisor-event senders. A child that
    // closes stdout and stderr before exiting ends both tasks, closing the
    // channel while the process is still alive: ordinary end of output, not
    // a supervision failure.
    let temp = tempfile::tempdir().unwrap();
    let program = script(temp.path(), "early-eof", "exec 1>&- 2>&-\nsleep 1");
    let (control, _handle) = HostCommandControl::new();
    let output = supervise_process_group(command(program), control, None)
        .await
        .expect("closing the pipes before exit is not a failure");
    assert_eq!(output.exit_code, Some(0));
}

#[tokio::test]
async fn supervisor_drains_large_stdout_and_stderr_without_deadlock() {
    let temp = tempfile::tempdir().unwrap();
    let program = script(
        temp.path(),
        "both-streams",
        "i=0; while [ $i -lt 1500 ]; do printf 'stdout-%04d-xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\\n' \"$i\"; printf 'stderr-%04d-yyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy\\n' \"$i\" >&2; i=$((i+1)); done",
    );
    let (control, _handle) = HostCommandControl::new();
    let output = supervise_process_group(command(program), control, None)
        .await
        .unwrap();

    assert_eq!(output.exit_code, Some(0));
    assert!(output.stdout.len() > 50_000);
    assert!(output.stderr.len() > 50_000);
    assert_eq!(output.stdout_bytes as usize, output.stdout.len());
    assert_eq!(output.stderr_bytes as usize, output.stderr.len());
}

/// A background descendant that marks itself started, then writes `late`
/// after `delay`. The foreground waits for the mark, so a missing `late` file
/// afterwards proves the group was killed, not that the descendant never ran.
struct Descendant {
    ready: PathBuf,
    late: PathBuf,
}

impl Descendant {
    fn new(dir: &Path) -> Self {
        Self {
            ready: dir.join("ready"),
            late: dir.join("late"),
        }
    }

    fn start(&self, delay: &str) -> String {
        format!(
            "(printf ready > '{ready}'; sleep {delay}; printf late > '{late}') &\nuntil [ -e '{ready}' ]; do sleep 0.01; done",
            ready = self.ready.display(),
            late = self.late.display(),
        )
    }

    async fn assert_killed(&self, after: Duration, cause: &str) {
        assert!(self.ready.exists(), "descendant never started ({cause})");
        tokio::time::sleep(after).await;
        assert!(!self.late.exists(), "descendant survived {cause}");
    }
}

#[tokio::test]
async fn stdout_overflow_terminates_group_and_prevents_late_mutation() {
    let temp = tempfile::tempdir().unwrap();
    let descendant = Descendant::new(temp.path());
    let body = format!(
        "{}\nwhile :; do printf '0123456789abcdef'; done",
        descendant.start("1")
    );
    let mut request = command(script(temp.path(), "overflow", &body));
    request.max_stdout_bytes = 256;
    let (control, _handle) = HostCommandControl::new();
    let error = supervise_process_group(request, control, None)
        .await
        .expect_err("overflow must fail operationally");

    assert!(
        error.to_string().contains("stdout output exceeded"),
        "{error}"
    );
    descendant
        .assert_killed(Duration::from_millis(1500), "output overflow")
        .await;
}

#[tokio::test]
async fn timeout_terminates_background_process_group() {
    let temp = tempfile::tempdir().unwrap();
    let descendant = Descendant::new(temp.path());
    let body = format!("{}\nsleep 30", descendant.start("2"));
    let mut request = command(script(temp.path(), "timeout", &body));
    // The descendant is started before the deadline and due after it.
    request.timeout_secs = 1;
    let (control, _handle) = HostCommandControl::new();
    // Issue #255: an operational outcome for the executor, not an error.
    let output = supervise_process_group(request, control, None)
        .await
        .unwrap();

    assert!(output.timed_out && output.exit_code.is_none());
    descendant
        .assert_killed(Duration::from_millis(1500), "timeout")
        .await;
}

#[tokio::test]
async fn pause_and_cancel_interrupt_and_reap_direct_child() {
    for signal in [HostCommandSignal::Paused, HostCommandSignal::Cancelled] {
        let temp = tempfile::tempdir().unwrap();
        let ready = temp.path().join("ready");
        let late = temp.path().join("late");
        let body = format!(
            "printf ready > '{}'; sleep 1; printf late > '{}'",
            ready.display(),
            late.display()
        );
        let program = script(temp.path(), "control", &body);
        let (control, handle) = HostCommandControl::new();
        let task = tokio::spawn(supervise_process_group(command(program), control, None));
        let started = std::time::Instant::now();
        while !ready.exists() {
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "child never started"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        handle.signal(signal).unwrap();
        let error = task.await.unwrap().expect_err("control must interrupt");
        assert!(error.to_string().contains(signal.as_str()), "{error}");
        tokio::time::sleep(Duration::from_millis(1500)).await;
        assert!(!late.exists(), "child survived {}", signal.as_str());
    }
}

#[test]
fn supervisor_clears_environment_and_delivers_exact_stdin() {
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "command::workflow_host_command_supervisor_tests::stdin_environment_child",
            "--nocapture",
        ])
        .env("ARCHON_R2A_AMBIENT_SENTINEL", "must-not-leak")
        .status()
        .unwrap();
    assert!(status.success());
}

#[tokio::test]
#[ignore = "isolated process environment"]
async fn stdin_environment_child() {
    let temp = tempfile::tempdir().unwrap();
    let program = script(
        temp.path(),
        "stdin-env",
        "printf 'declared=%s\\n' \"${DECLARED:-missing}\"; printf 'ambient=%s\\n' \"${ARCHON_R2A_AMBIENT_SENTINEL:-absent}\"; cat",
    );
    let mut request = command(program);
    request
        .environment
        .insert("DECLARED".into(), "allowed".into());
    request.stdin = Some(b"opaque;$(printf not-executed)".to_vec());
    let (control, _handle) = HostCommandControl::new();
    let output = supervise_process_group(request, control, None)
        .await
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();

    assert!(stdout.contains("declared=allowed"));
    assert!(stdout.contains("ambient=absent"));
    assert!(stdout.ends_with("opaque;$(printf not-executed)"));
    assert!(!stdout.contains("must-not-leak"));
}

#[path = "workflow_host_command_supervisor_tree_tests.rs"]
mod tree;

#[path = "workflow_host_command_supervisor_descriptor_tests.rs"]
mod descriptors;

#[path = "workflow_host_command_supervisor_limit_tests.rs"]
mod limit;

#[path = "workflow_host_command_idle_tests.rs"]
mod idle;
