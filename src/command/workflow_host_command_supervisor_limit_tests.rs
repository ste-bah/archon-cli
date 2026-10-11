//! Output limits bound display memory; they never change a child outcome.

use std::time::{Duration, Instant};

use archon_workflow::WorkflowResult;

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
    // Hold the runtime thread until the child exits so stdin delivery and exit
    // are both ready when the supervisor resumes.
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
            "fixture never exited"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    task.await.unwrap()
}

#[tokio::test]
async fn huge_output_is_spilled_fully_and_exit_status_and_json_are_preserved() {
    const LARGE: usize = 6 * 1024 * 1024;
    let temp = tempfile::tempdir().unwrap();
    let body = format!(
        "head -c {LARGE} /dev/zero | tr '\\000' ' '; printf '{{\"verdict\":\"accepted\"}}'; head -c {LARGE} /dev/zero | tr '\\000' 'e' >&2; exit 7"
    );
    let mut request = command(script(temp.path(), "huge-output", &body));
    request.max_stdout_bytes = 256;
    request.max_stderr_bytes = 256;

    let (control, _handle) = HostCommandControl::new();
    let output = supervise_process_group(request, control, None)
        .await
        .unwrap();
    assert_eq!(
        output.exit_code,
        Some(7),
        "the child was allowed to exit normally"
    );
    assert!(!output.timed_out);

    let mut expected_stdout = vec![b' '; LARGE];
    expected_stdout.extend_from_slice(br#"{"verdict":"accepted"}"#);
    let expected_stderr = vec![b'e'; LARGE];
    assert_eq!(
        std::fs::read(output.stdout_path.as_ref().unwrap()).unwrap(),
        expected_stdout
    );
    assert_eq!(
        std::fs::read(output.stderr_path.as_ref().unwrap()).unwrap(),
        expected_stderr
    );
    let parsed: serde_json::Value =
        serde_json::from_slice(&std::fs::read(output.stdout_path.as_ref().unwrap()).unwrap())
            .unwrap();
    assert_eq!(parsed["verdict"], "accepted");

    assert!(output.stdout.len() < 512);
    assert!(output.stderr.len() < 512);
    for (view, stream) in [(&output.stdout, "stdout"), (&output.stderr, "stderr")] {
        let view = String::from_utf8_lossy(view);
        assert!(view.starts_with(if stream == "stdout" {
            "            "
        } else {
            "eeee"
        }));
        assert!(view.contains("output truncated: "));
        assert!(view.contains("bytes omitted; full output: host-command-results/"));
        assert!(view.contains(&format!("/{stream}.bin")));
    }
}

#[tokio::test]
async fn output_at_the_cap_is_complete_and_unmarked() {
    let temp = tempfile::tempdir().unwrap();
    let mut request = command(script(
        temp.path(),
        "at-cap",
        "head -c 256 /dev/zero | tr '\\000' x\nexit 0",
    ));
    request.max_stdout_bytes = 256;
    let (control, _handle) = HostCommandControl::new();
    let output = supervise_process_group(request, control, None)
        .await
        .unwrap();
    assert_eq!(output.exit_code, Some(0));
    assert_eq!(output.stdout, vec![b'x'; 256]);
    assert_eq!(output.truncation(), (false, false));
}

#[tokio::test]
async fn output_one_byte_over_the_cap_is_reported_as_truncated() {
    let temp = tempfile::tempdir().unwrap();
    let mut request = command(script(
        temp.path(),
        "over-cap",
        "head -c 257 /dev/zero | tr '\\000' x",
    ));
    request.max_stdout_bytes = 256;
    let (control, _handle) = HostCommandControl::new();
    let output = supervise_process_group(request, control, None)
        .await
        .unwrap();
    assert_eq!(output.stdout_bytes, 257);
    assert_eq!(output.truncation(), (true, false));
}

#[tokio::test]
async fn truncated_multibyte_output_keeps_the_view_utf8_and_spill_exact() {
    let temp = tempfile::tempdir().unwrap();
    let expected = format!("{}x", "é".repeat(200));
    let body = format!("printf '%s' '{expected}'");
    let mut request = command(script(temp.path(), "multibyte-over-cap", &body));
    // This odd cap splits a two-byte character at both retained edges.
    request.max_stdout_bytes = 257;
    let (control, _handle) = HostCommandControl::new();
    let output = supervise_process_group(request, control, None)
        .await
        .expect("multibyte output beyond the cap remains a successful call");

    assert_eq!(output.exit_code, Some(0));
    assert!(output.stdout_truncated);
    assert!(std::str::from_utf8(&output.stdout).is_ok());
    assert_eq!(
        std::fs::read(output.stdout_path.as_ref().unwrap()).unwrap(),
        expected.as_bytes()
    );
}

#[tokio::test]
async fn two_attempts_of_one_call_keep_separate_full_spills() {
    let temp = tempfile::tempdir().unwrap();
    let call_dir = temp.path().join("host-command-results/call-123");
    for (attempt, text) in [(1, "first attempt"), (2, "second attempt")] {
        let mut request = command(script(
            temp.path(),
            &format!("attempt-{attempt}"),
            &format!("printf '%s' '{text}'"),
        ));
        request.spill_dir = Some(call_dir.join(format!("attempt-{attempt}")));
        request.max_stdout_bytes = 4;
        let (control, _handle) = HostCommandControl::new();
        let output = supervise_process_group(request, control, None)
            .await
            .unwrap();
        assert_eq!(
            std::fs::read(output.stdout_path.as_ref().unwrap()).unwrap(),
            text.as_bytes()
        );
        assert!(
            output
                .stdout_path
                .as_ref()
                .unwrap()
                .ends_with(format!("attempt-{attempt}/stdout.bin"))
        );
    }
}

#[cfg(unix)]
#[tokio::test]
async fn spill_creation_refuses_symlinked_directories_and_files() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("target");
    std::fs::create_dir(&target).unwrap();
    let directory_link = temp.path().join("host-command-results");
    symlink(&target, &directory_link).unwrap();
    let mut request = command(script(temp.path(), "dir-link", "printf x"));
    request.spill_dir = Some(directory_link.join("call"));
    let (control, _handle) = HostCommandControl::new();
    assert!(
        supervise_process_group(request, control, None)
            .await
            .is_err()
    );
    assert!(!target.join("call").exists());

    let results = temp.path().join("real-results");
    let outside_call = temp.path().join("outside-call");
    std::fs::create_dir(&results).unwrap();
    std::fs::create_dir(&outside_call).unwrap();
    symlink(&outside_call, results.join("call")).unwrap();
    let mut request = command(script(temp.path(), "call-link", "printf x"));
    request.spill_dir = Some(results.join("call/attempt-1-execution"));
    let (control, _handle) = HostCommandControl::new();
    assert!(
        supervise_process_group(request, control, None)
            .await
            .is_err()
    );
    assert!(std::fs::read_dir(&outside_call).unwrap().next().is_none());

    let real_dir = temp.path().join("real-call");
    std::fs::create_dir(&real_dir).unwrap();
    let sentinel = temp.path().join("sentinel");
    std::fs::write(&sentinel, b"keep").unwrap();
    symlink(&sentinel, real_dir.join("stdout.bin")).unwrap();
    let mut request = command(script(temp.path(), "file-link", "printf x"));
    request.spill_dir = Some(real_dir);
    let (control, _handle) = HostCommandControl::new();
    assert!(
        supervise_process_group(request, control, None)
            .await
            .is_err()
    );
    assert_eq!(std::fs::read(sentinel).unwrap(), b"keep");
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
        stdout_retained_bytes: 4,
        stderr_retained_bytes: 2,
        stdout_truncated: true,
        stderr_truncated: false,
        stdout_path: None,
        stderr_path: None,
    };
    assert_eq!(output.truncation(), (true, false));
}

#[tokio::test]
async fn literal_truncation_marker_under_the_cap_is_content_not_metadata() {
    let temp = tempfile::tempdir().unwrap();
    let literal = "\noutput truncated: this is child content";
    let mut request = command(script(
        temp.path(),
        "literal-marker",
        &format!("printf '%s' '{literal}'"),
    ));
    request.max_stdout_bytes = 256;
    let (control, _handle) = HostCommandControl::new();
    let output = supervise_process_group(request, control, None)
        .await
        .unwrap();
    assert_eq!(output.stdout, literal.as_bytes());
    assert_eq!(output.stdout_retained_bytes, literal.len() as u64);
    assert_eq!(output.stdout_bytes, literal.len() as u64);
    assert_eq!(output.truncation(), (false, false));
}

#[tokio::test]
async fn stdin_the_child_stops_reading_fails_the_call_when_the_exit_is_seen_first() {
    // The child takes one byte of a payload far larger than a pipe holds,
    // then exits 0. The rest of the payload was never delivered.
    let temp = tempfile::tempdir().unwrap();
    let body = "sleep 0.5\nhead -c 1 >/dev/null\nexit 0";
    let mut request = command(script(temp.path(), "short-read", body));
    request.stdin = Some(vec![b'x'; 4 * 1024 * 1024]);
    let error = exit_seen_before_events(request)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("stdin delivery failed"), "{error}");
}
