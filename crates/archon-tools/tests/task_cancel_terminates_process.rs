//! Cancelling a task kills the work, not just a flag (#189 Phase 9).
//!
//! The tasks overlay's acceptance criterion is that cancelling *stops the
//! underlying work* — a test asserting a token flipped would pass whether or
//! not anything died, which is precisely the class of green-but-hollow check
//! this issue exists to remove.
//!
//! The path under test has three links:
//!
//! 1. `TaskManager::stop_task` fires the task's execution token.
//! 2. That same token is what `TaskCreate` hands the subagent runner
//!    (`task_create.rs`), and it arrives as `ToolContext::cancel_parent`.
//! 3. `BashTool` selects on `cancel_parent.cancelled()`, so firing it ends the
//!    command and takes the process with it.
//!
//! Link 2 needs a live model to exercise end to end, so it is asserted
//! structurally: the token `stop_task` fires is the one `execution_token`
//! hands out. Links 1 and 3 are exercised against real processes.

use std::time::Duration;

use archon_tools::task_manager::TaskManager;
use tokio_util::sync::CancellationToken;

// Only the process-level test uses the Bash machinery, and it is unix-only.
#[cfg(unix)]
use archon_tools::bash::BashTool;
#[cfg(unix)]
use archon_tools::tool::{Tool, ToolContext};
#[cfg(unix)]
use serde_json::json;

/// Link 1 + 2: the token `stop_task` fires is the one handed to the runner.
#[test]
fn stopping_a_task_fires_the_token_the_runner_was_given() {
    let manager = TaskManager::new();
    let id = manager.create_task("cancellable work");
    let runner_token = manager
        .execution_token(&id)
        .expect("a new task has an execution token");

    assert!(
        !runner_token.is_cancelled(),
        "a fresh task must not start cancelled"
    );

    manager.stop_task(&id).expect("stop the task");

    assert!(
        runner_token.is_cancelled(),
        "stop_task must fire the very token TaskCreate passes to run_subagent"
    );
}

/// Cancelling a parent cascades to a child task, so stopping a parent does not
/// leave its dispatched work running.
#[test]
fn stopping_a_parent_cancels_the_child_task_token() {
    let manager = TaskManager::new();
    let parent = CancellationToken::new();
    let id = manager.create_task_with_parent("child work", Some(&parent));
    let child_token = manager.execution_token(&id).expect("execution token");

    parent.cancel();

    assert!(child_token.is_cancelled());
}

/// Link 3, against a real process: a command still running when its
/// `cancel_parent` fires is ended, and its child does not outlive it.
///
/// Unix-only because the assertion is "this pid is gone", and the existing
/// process probes in this crate are `/proc`- and `kill -0`-based.
#[tokio::test]
#[cfg(unix)]
async fn cancelling_the_context_token_terminates_the_running_process() {
    let dir = tempfile::tempdir().expect("tempdir");
    let pid_file = dir.path().join("cancelled-child.pid");
    let cancel = CancellationToken::new();

    let tool = BashTool {
        timeout_secs: 30,
        max_output_bytes: 1024,
        ..Default::default()
    };
    let ctx = ToolContext {
        working_dir: dir.path().to_path_buf(),
        cancel_parent: Some(cancel.clone()),
        ..ToolContext::default()
    };

    // Record the child's pid, then block long enough that only cancellation can
    // end it — a command that exits on its own would prove nothing.
    let command = format!(
        "sh -c 'echo $$ > {0}; exec sleep 60' & while [ ! -s {0} ]; do sleep 0.01; done; wait",
        shell_quote(&pid_file)
    );

    let cancel_for_task = cancel.clone();
    let pid_path = pid_file.clone();
    tokio::spawn(async move {
        // Fire only once the child exists, so the test cannot pass by
        // cancelling before anything was ever spawned.
        for _ in 0..500 {
            if std::fs::read_to_string(&pid_path)
                .map(|pid| !pid.trim().is_empty())
                .unwrap_or(false)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        cancel_for_task.cancel();
    });

    let result = tokio::time::timeout(Duration::from_secs(10), tool.execute(json!({"command": command}), &ctx))
        .await
        .expect("cancellation must end the command well inside the 30s timeout");

    assert!(
        result.is_error,
        "a cancelled command reports failure: {}",
        result.content
    );

    let pid = std::fs::read_to_string(&pid_file).expect("child pid file");
    wait_until_process_is_absent(pid.trim()).await;
}

#[cfg(unix)]
fn shell_quote(path: &std::path::Path) -> String {
    format!(
        "'{}'",
        path.display().to_string().replace('\'', "'\\\"'\\\"'")
    )
}

/// Bounded poll rather than an immediate check: SIGKILL is delivered to the
/// process group and reaped asynchronously, so a live pid for a few
/// milliseconds after `execute` returns is teardown latency, not survival.
#[cfg(unix)]
async fn wait_until_process_is_absent(pid: &str) {
    for _ in 0..60 {
        if !process_exists(pid) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let _ = std::process::Command::new("kill")
        .arg("-9")
        .arg(pid)
        .status();
    panic!("process survived task cancellation: pid={pid}");
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
