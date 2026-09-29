//! Issue-134: pausing a run ends every process an in-flight agent Bash call
//! started, before the call is recorded as ended.
//!
//! Live, a pause abandoned a unit's call (`control_race` drops its future)
//! and its `sh <check>.sh -> cargo run` child ran on for 1h42m, re-parented
//! to launchd, ignoring SIGTERM: dropping the call SIGKILLed only the Bash
//! wrapper, the leader of the command's process group. Here a Bash call
//! starts a detached grandchild that ignores SIGTERM, the run is paused
//! through the real control path, and by the time `until_run_stops` returns
//! the grandchild must be gone. On 25ff60622 it survives.
#![cfg(unix)]

use std::path::Path;
use std::time::{Duration, Instant};

use archon_tools::tool::{Tool, ToolContext};
use archon_workflow::control_race::until_run_stops;
use archon_workflow::{
    LifecycleAction, LifecycleController, WorkflowError, WorkflowSpec, WorkflowStore,
};

fn spec() -> WorkflowSpec {
    WorkflowSpec {
        schema: archon_workflow::spec::WORKFLOW_SCHEMA.to_string(),
        name: "issue-134".to_string(),
        task: "test".to_string(),
        target_repository_root: None,
        max_parallelism: 1,
        max_agents: 1,
        stages: Vec::new(),
        permissions: Default::default(),
        learning_hooks: Vec::new(),
    }
}

/// A process that is gone: no such pid, or only its zombie is left.
fn alive(pid: i32) -> bool {
    if unsafe { libc::kill(pid, 0) } != 0 {
        return false;
    }
    let stat = std::process::Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    !stat.is_empty() && !stat.starts_with('Z')
}

fn read_pid(path: &Path) -> Option<i32> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pause_ends_a_bash_calls_detached_grandchild_that_ignores_sigterm() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let run = store.create_run(spec()).unwrap();
    let pidfile = temp.path().join("grandchild.pid");
    // The grandchild ignores TERM, detaches from its parent's wait, and
    // records itself; the call then blocks on it, as a long build would.
    let command = format!(
        "nohup sh -c 'trap \"\" TERM; echo $$ > {pid}; exec sleep 600' >/dev/null 2>&1 & wait",
        pid = pidfile.display()
    );
    let tool = archon_tools::bash::BashTool::default();
    let ctx = ToolContext {
        working_dir: temp.path().to_path_buf(),
        ..Default::default()
    };
    let call = async {
        let result = tool
            .execute(
                serde_json::json!({ "command": command, "timeout": 600000 }),
                &ctx,
            )
            .await;
        Ok::<_, WorkflowError>(result)
    };
    let pauser = {
        let store = store.clone();
        let run_id = run.id.clone();
        let pidfile = pidfile.clone();
        tokio::spawn(async move {
            let started = Instant::now();
            while read_pid(&pidfile).is_none() && started.elapsed() < Duration::from_secs(30) {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            LifecycleController::new(store)
                .apply(&run_id, LifecycleAction::Pause)
                .expect("pause the run");
        })
    };
    let outcome = until_run_stops(&store, &run.id, "bash-call", call).await;
    pauser.await.unwrap();
    let pid = read_pid(&pidfile).expect("the grandchild started");
    // Anything still alive is killed here, pass or fail: no test leaves a
    // process behind.
    let survived = alive(pid);
    if survived {
        unsafe {
            libc::kill(pid, libc::SIGKILL);
        }
    }
    assert!(
        matches!(outcome, Err(WorkflowError::ControlPaused(_))),
        "the pause must abandon the call: {outcome:?}"
    );
    assert!(
        !survived,
        "the call's detached grandchild {pid} survived the pause that ended the call"
    );
}

/// When the call ran its tool in a spawned task (a turn's tools fanned out
/// in parallel), dropping the call only ABORTS that task: its Bash future is
/// dropped later, on another thread. The host ends the run's trees itself
/// before it records the call, and nothing is left for the late drop.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_host_ends_a_runs_trees_before_recording_even_when_a_tool_task_is_still_unwinding() {
    let temp = tempfile::tempdir().unwrap();
    let pidfile = temp.path().join("grandchild.pid");
    let command = format!(
        "nohup sh -c 'trap \"\" TERM; echo $$ > {pid}; exec sleep 600' >/dev/null 2>&1 & wait",
        pid = pidfile.display()
    );
    let ctx = ToolContext {
        working_dir: temp.path().to_path_buf(),
        session_id: "wf-issue-134-run".to_string(),
        ..Default::default()
    };
    let mut tasks = tokio::task::JoinSet::new();
    tasks.spawn(async move {
        archon_tools::bash::BashTool::default()
            .execute(
                serde_json::json!({ "command": command, "timeout": 600000 }),
                &ctx,
            )
            .await
    });
    let started = Instant::now();
    while read_pid(&pidfile).is_none() && started.elapsed() < Duration::from_secs(30) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let pid = read_pid(&pidfile).expect("the grandchild started");
    tasks.abort_all();
    let survivors = archon_tools::bash::end_process_groups_of("wf-issue-134-run");
    let survived = alive(pid);
    if survived {
        unsafe {
            libc::kill(pid, libc::SIGKILL);
        }
    }
    drop(tasks);
    assert!(survivors.is_empty(), "{survivors:?}");
    assert!(!survived, "grandchild {pid} outlived the host's teardown");
}
