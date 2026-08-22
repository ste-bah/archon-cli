use std::time::Duration;

use super::*;
use crate::tool::ToolContext;

#[tokio::test]
#[cfg(unix)]
async fn parent_exit_with_descendant_held_pipes_is_cleaned_before_result() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("held-pipes.pid");
    let tool = BashTool {
        timeout_secs: 1,
        max_output_bytes: 1024,
        ..Default::default()
    };
    let command = descendant_holding_pipes_command(&pid_file);
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        tool.execute(
            json!({"command": command, "timeout": 1_000}),
            &ToolContext {
                working_dir: dir.path().to_path_buf(),
                ..ToolContext::default()
            },
        ),
    )
    .await
    .expect("Bash invocation exceeded its strict outer deadline");

    assert!(
        !result.is_error,
        "completed shell with cleaned descendants should succeed: {}",
        result.content
    );
    wait_until_process_is_absent(&std::fs::read_to_string(&pid_file).expect("descendant pid file"))
        .await;
}

#[cfg(unix)]
fn descendant_holding_pipes_command(pid_file: &std::path::Path) -> String {
    let path = shell_quote(pid_file);
    format!(
        "sh -c \"trap '' HUP; echo \\$\\$ > {path}; exec sleep 30\" & while [ ! -s {path} ]; do sleep 0.01; done"
    )
}

#[cfg(unix)]
fn shell_quote(path: &std::path::Path) -> String {
    format!(
        "'{}'",
        path.display().to_string().replace('\'', "'\\\"'\\\"'")
    )
}

/// The containment this module is named for.
///
/// Until #192 the child inherited the caller's process group, so the pgid the
/// cleanup path signals was a plain pid that usually named no group — the kill
/// hit nothing and returned ESRCH, which reads as "already gone". When the pid
/// did collide with a real group it named someone else's processes; macOS
/// refused that with EPERM and CI went red.
#[tokio::test]
#[cfg(unix)]
async fn bash_runs_in_a_process_group_of_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let tool = BashTool::default();
    let result = tool
        .execute(
            json!({"command": "ps -o pgid= -p $$"}),
            &ToolContext {
                working_dir: dir.path().to_path_buf(),
                ..ToolContext::default()
            },
        )
        .await;

    assert!(!result.is_error, "{}", result.content);
    let child_group: i32 = result
        .content
        .lines()
        .filter_map(|line| line.trim().parse::<i32>().ok())
        .next_back()
        .unwrap_or_else(|| panic!("no process group in output: {}", result.content));
    let own_group = unsafe { libc::getpgrp() };

    assert_ne!(
        child_group, own_group,
        "the child shares this process's group, so killing its 'group' would \
         either do nothing or signal processes archon does not own"
    );
    assert!(child_group > 0, "implausible pgid {child_group}");
}

/// The session that keeps a prompting command from wedging the wave (#197).
///
/// A process group still inherits the controlling terminal, so a command that
/// wants an answer opens `/dev/tty` directly and gets one no matter where stdin
/// points. Doing that from a background group raises SIGTTIN, and SIGTTIN
/// *stops* the child rather than failing it: state `T`, pipes still held,
/// nothing there to answer. One `git diff HEAD | patch -p1` that could not place
/// a hunk asked `File to patch:` and held that state for thirty minutes, with an
/// eleven-item parallel wave behind it. `setsid` leaves the child no controlling
/// terminal at all, so the open fails with ENXIO and the command errors out
/// promptly -- which lets the agent read a real error and try something else.
///
/// Asserted as "a session of its own" rather than "opening /dev/tty fails",
/// because a CI runner has no controlling terminal to begin with: the open would
/// fail there whatever this code did, and the test would pass without testing
/// anything. A session id cannot go vacuous the same way -- the parent has one
/// either way, so a child that skipped `setsid` would share it and fail here.
///
/// Linux-only because `ps -o sid=` is. BSD `ps` on macOS reports a session
/// pointer under `sess`, which is not comparable with `getsid`.
#[tokio::test]
#[cfg(target_os = "linux")]
async fn bash_runs_in_a_session_of_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let tool = BashTool::default();
    let result = tool
        .execute(
            json!({"command": "ps -o sid= -p $$"}),
            &ToolContext {
                working_dir: dir.path().to_path_buf(),
                ..ToolContext::default()
            },
        )
        .await;

    assert!(!result.is_error, "{}", result.content);
    let child_session: i32 = result
        .content
        .lines()
        .filter_map(|line| line.trim().parse::<i32>().ok())
        .next_back()
        .unwrap_or_else(|| panic!("no session id in output: {}", result.content));
    let own_session = unsafe { libc::getsid(0) };

    assert_ne!(
        child_session, own_session,
        "the child shares this process's session, so it still has our \
         controlling terminal and a command that prompts on /dev/tty will be \
         stopped by SIGTTIN instead of failing"
    );
    assert!(child_session > 0, "implausible sid {child_session}");
}

#[tokio::test]
#[cfg(unix)]
async fn normal_completion_kills_delayed_background_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let delayed_file = dir.path().join("delayed-write");
    let pid_file = dir.path().join("delayed-child.pid");
    let tool = BashTool {
        max_output_bytes: 1024,
        ..Default::default()
    };

    // The parent waits until the child is genuinely stopped before finishing.
    //
    // Without that wait the parent could exit while the child was still on its
    // way to `kill -STOP`, so cleanup met a running child on a fast machine and
    // a stopped one on a slow machine — two different scenarios under one
    // assertion, and the slow one failed. Synchronising on the state makes the
    // case under test the same every run instead of a function of load.
    let result = tool
        .execute(
            json!({
                "command": format!(
                    "sh -c 'kill -STOP \"$$\"; printf delayed > {}' & child=$!; printf '%s' \"$child\" > {}; \
                     while kill -0 \"$child\" 2>/dev/null && ! ps -o stat= -p \"$child\" | grep -q '[Tt]'; do sleep 0.01; done",
                    shell_quote(&delayed_file),
                    shell_quote(&pid_file),
                )
            }),
            &ToolContext {
                working_dir: dir.path().to_path_buf(),
                ..ToolContext::default()
            },
        )
        .await;

    assert!(!result.is_error, "{}", result.content);
    let pid = std::fs::read_to_string(&pid_file).expect("fixture child pid");
    wait_until_process_is_absent(&pid).await;
    assert!(
        !delayed_file.exists(),
        "a detached mutation escaped Bash completion"
    );
}

#[tokio::test]
async fn stderr_redirection_and_heredoc_are_not_background_commands() {
    let dir = tempfile::tempdir().unwrap();
    let tool = BashTool::default();
    let result = tool
        .execute(
            json!({"command": "cat <<'EOF' >&2\nheredoc stderr\nEOF"}),
            &ToolContext {
                working_dir: dir.path().to_path_buf(),
                ..ToolContext::default()
            },
        )
        .await;

    assert!(!result.is_error, "{}", result.content);
    assert!(result.content.contains("heredoc stderr"));
}

#[tokio::test]
#[cfg(unix)]
async fn parent_completion_returns_after_descendant_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("cleanup-before-return.pid");
    let tool = BashTool {
        timeout_secs: 1,
        max_output_bytes: 1024,
        ..Default::default()
    };
    let result = tool
        .execute(
            json!({"command": descendant_holding_pipes_command(&pid_file), "timeout": 1_000}),
            &ToolContext {
                working_dir: dir.path().to_path_buf(),
                ..ToolContext::default()
            },
        )
        .await;

    assert!(!result.is_error, "{}", result.content);
    let pid = std::fs::read_to_string(pid_file).unwrap();
    // Bounded poll rather than an immediate check.
    //
    // `terminate_child` SIGKILLs the process GROUP and then waits for the direct
    // child. The descendant is signalled but reaped asynchronously by the
    // kernel, so it can still be visible for a few milliseconds after `execute`
    // returns. That window is invisible on an idle machine and reliably lost
    // under CI load, which is what made this test flaky on ubuntu.
    //
    // The guarantee worth asserting is that the descendant does not SURVIVE --
    // SIGKILL cannot be trapped, so anything still present is mid-teardown, not
    // leaked. This still fails if cleanup never happens, which is the regression
    // the test exists to catch; it only tolerates teardown latency.
    wait_until_process_is_absent(&pid).await;
}

#[tokio::test]
#[cfg(unix)]
async fn timeout_kills_background_process_group() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("child.pid");
    let tool = BashTool {
        timeout_secs: 1,
        max_output_bytes: 1024,
        ..Default::default()
    };
    let result = tool
        .execute(
            json!({
                "command": format!(
                    "sh -c 'trap \"\" TERM; while :; do sleep 1; done' & echo $! > {}; wait",
                    pid_file.display()
                ),
                "timeout": 100
            }),
            &ToolContext {
                working_dir: dir.path().to_path_buf(),
                ..ToolContext::default()
            },
        )
        .await;

    assert!(result.is_error, "command should time out");
    let pid = std::fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .to_string();
    for _ in 0..20 {
        if !process_exists(&pid) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let _ = std::process::Command::new("kill")
        .arg("-9")
        .arg(&pid)
        .status();
    panic!("background sleep process survived Bash timeout: pid={pid}");
}

#[cfg(unix)]
async fn wait_until_process_is_absent(pid: &str) {
    let pid = pid.trim();
    for _ in 0..40 {
        if !process_exists(pid) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("background process survived Bash timeout: pid={pid}");
}

#[cfg(unix)]
fn process_exists(pid: &str) -> bool {
    if std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|stat| {
            stat.rsplit_once(") ")
                .map(|(_, fields)| fields.starts_with('Z'))
        })
        .unwrap_or(false)
    {
        return false;
    }
    std::process::Command::new("kill")
        .arg("-0")
        .arg(pid)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}
/// The live wedge. A program that wants to ask the operator something opens
/// `/dev/tty` rather than reading stdin, so `Stdio::null()` does not stop it.
/// From a background process group that raises SIGTTIN and the child is
/// *stopped* — state `T`, pipes held, nothing to answer it. An agent's
/// `git diff HEAD | patch -p1` prompted "File to patch:" and sat there for 30
/// minutes, freezing an eleven-item wave behind it.
///
/// In its own session there is no controlling terminal, so the open fails and
/// the command returns promptly with an error the agent can act on. This must
/// finish well inside the tool's own timeout: the point is that it fails,
/// not that something eventually kills it.
#[tokio::test]
#[cfg(unix)]
async fn a_command_that_wants_the_terminal_fails_instead_of_stopping() {
    let dir = tempfile::tempdir().unwrap();
    let tool = BashTool {
        timeout_secs: 30,
        max_output_bytes: 4096,
        ..Default::default()
    };
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        tool.execute(
            // Reading the controlling terminal directly is what `patch`, `ssh`
            // and `git` prompts all do.
            json!({ "command": "read -r line < /dev/tty; echo \"got:$line\"" }),
            &ToolContext {
                working_dir: dir.path().to_path_buf(),
                ..ToolContext::default()
            },
        ),
    )
    .await
    .expect("a terminal read must not hang the tool");

    // The open fails with ENXIO — "Device not configured" on macOS, "No such
    // device or address" on Linux — rather than the process being stopped.
    let content = result.content.to_ascii_lowercase();
    assert!(
        content.contains("/dev/tty")
            && (content.contains("device not configured") || content.contains("no such device")),
        "the terminal open must fail outright: {}",
        result.content
    );
    assert!(
        !result.content.contains("got:x"),
        "no terminal input can have been read: {}",
        result.content
    );
}
