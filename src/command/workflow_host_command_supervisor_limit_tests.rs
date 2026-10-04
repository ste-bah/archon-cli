//! Issue 272: an output overflow or a stdin failure decides the call however
//! the child's exit and the supervisor's events are ordered.
//!
//! Each fixture sleeps, then breaks its limit and exits at once, while the
//! test holds the runtime's only thread. The test releases the thread only
//! once it has seen the child exit, so the exit and the event are both ready
//! and the select polls the exit first: the order that used to publish an
//! overflowing call.
use std::time::{Duration, Instant};

use archon_workflow::{WorkflowError, WorkflowResult};

use super::super::workflow_host_command_catalog::ResolvedHostCommand;
use super::super::workflow_host_command_supervisor::{
    HostCommandControl, SupervisedProcessOutput, supervise_process_group,
};
use super::{command, script};

async fn exit_seen_before_events(
    request: ResolvedHostCommand,
) -> WorkflowResult<SupervisedProcessOutput> {
    // The fixture records its pid first, so the test can see it exit.
    let fixture = std::path::PathBuf::from(&request.args[0]);
    let pid_file = fixture.with_extension("pid");
    let body = std::fs::read_to_string(&fixture).unwrap().replacen(
        "set -eu\n",
        &format!("set -eu\nprintf '%s' \"$$\" > '{}'\n", pid_file.display()),
        1,
    );
    std::fs::write(&fixture, body).unwrap();
    let (control, _handle) = HostCommandControl::new();
    let task = tokio::spawn(supervise_process_group(request, control, None));
    // Let the supervisor spawn the child and reach its select.
    tokio::time::sleep(Duration::from_millis(250)).await;
    // The child breaks its limit and exits while nothing else can run: hold
    // the thread until it has exited (a zombie or gone, never running).
    let start = Instant::now();
    loop {
        let pid = std::fs::read_to_string(&pid_file)
            .ok()
            .and_then(|text| text.trim().parse::<u32>().ok());
        if pid.is_some_and(|pid| archon_shell::process_tree::start_of(pid).is_none()) {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "the fixture never exited"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    task.await.unwrap()
}

fn failure(result: WorkflowResult<SupervisedProcessOutput>) -> String {
    match result {
        Err(WorkflowError::StageFailed(message)) => message,
        other => panic!("expected the call to fail, got {other:?}"),
    }
}

#[tokio::test]
async fn stdout_overflow_fails_the_call_when_the_exit_is_seen_first() {
    let temp = tempfile::tempdir().unwrap();
    let body = "sleep 0.5\nhead -c 300 /dev/zero\nexit 0";
    let mut request = command(script(temp.path(), "overflow-exit", body));
    request.max_stdout_bytes = 256;
    let message = failure(exit_seen_before_events(request).await);
    assert!(
        message.contains("stdout output exceeded 256 bytes"),
        "{message}"
    );
}

#[tokio::test]
async fn stderr_overflow_by_one_byte_fails_the_call_when_the_exit_is_seen_first() {
    let temp = tempfile::tempdir().unwrap();
    let body = "sleep 0.5\nhead -c 257 /dev/zero >&2\nexit 0";
    let mut request = command(script(temp.path(), "overflow-stderr", body));
    request.max_stderr_bytes = 256;
    let message = failure(exit_seen_before_events(request).await);
    assert!(
        message.contains("stderr output exceeded 256 bytes"),
        "{message}"
    );
}

#[tokio::test]
async fn overflow_then_a_failing_exit_is_the_overflow_not_a_returned_exit_code() {
    let temp = tempfile::tempdir().unwrap();
    let body = "sleep 0.5\nhead -c 300 /dev/zero\nexit 3";
    let mut request = command(script(temp.path(), "overflow-fail", body));
    request.max_stdout_bytes = 256;
    let message = failure(exit_seen_before_events(request).await);
    assert!(message.contains("stdout output exceeded"), "{message}");
}

#[tokio::test]
async fn output_exactly_at_the_limit_is_complete_and_untruncated() {
    let temp = tempfile::tempdir().unwrap();
    let body = "sleep 0.5\nhead -c 256 /dev/zero\nexit 0";
    let mut request = command(script(temp.path(), "at-limit", body));
    request.max_stdout_bytes = 256;
    let output = exit_seen_before_events(request).await.unwrap();
    assert_eq!(output.exit_code, Some(0));
    assert_eq!((output.stdout.len(), output.stdout_bytes), (256, 256));
    assert_eq!(output.truncation(), (false, false));
}

#[test]
fn truncation_is_reported_from_the_bytes_written_not_assumed_false() {
    let output = SupervisedProcessOutput {
        exit_code: None,
        timed_out: true,
        stdout: vec![b'x'; 4],
        stderr: vec![b'y'; 2],
        stdout_bytes: 10,
        stderr_bytes: 2,
    };
    assert_eq!(output.truncation(), (true, false));
}

#[tokio::test]
async fn stdin_the_child_stops_reading_fails_the_call_when_the_exit_is_seen_first() {
    // The child takes one byte of a payload far larger than a pipe holds,
    // then exits 0. The rest of the payload was never delivered.
    let temp = tempfile::tempdir().unwrap();
    let body = "sleep 0.5\nhead -c 1 >/dev/null\nexit 0";
    let mut request = command(script(temp.path(), "short-read", body));
    request.stdin = Some(vec![b'x'; 4 * 1024 * 1024]);
    let message = failure(exit_seen_before_events(request).await);
    assert!(message.contains("stdin delivery failed"), "{message}");
}
