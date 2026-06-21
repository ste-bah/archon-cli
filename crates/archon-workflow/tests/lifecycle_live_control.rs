use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use archon_workflow::{
    LifecycleAction, LifecycleController, RunStatus, StageRunOutput, StageRunRequest, StageStatus,
    WorkflowExecutor, WorkflowPolicy, WorkflowSpec, WorkflowStageRunner, WorkflowStore,
};

fn fanout_spec() -> WorkflowSpec {
    WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: lifecycle-control-fanout
task: pause during fanout
max_parallelism: 2
stages:
  - id: discover
    kind: agent
    agent: workflow-discovery
    outputs: [items]
  - id: review
    kind: fanout
    agent: workflow-reviewer
    foreach: ${discover.items}
    depends_on: [discover]
"#,
    )
    .unwrap()
}

struct SlowFanoutRunner {
    launched: Arc<AtomicUsize>,
}

impl archon_workflow::WriteBoundaryProbe for SlowFanoutRunner {}

#[async_trait::async_trait]
impl WorkflowStageRunner for SlowFanoutRunner {
    async fn run_stage(
        &self,
        request: StageRunRequest,
    ) -> archon_workflow::WorkflowResult<StageRunOutput> {
        if request.stage_id == "discover" {
            let items = (0..20)
                .map(|idx| format!(r#"{{"unit":"u{idx}"}}"#))
                .collect::<Vec<_>>()
                .join(",");
            return Ok(StageRunOutput::markdown(format!(
                r#"{{"items":[{items}]}}"#
            )));
        }
        self.launched.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        Ok(StageRunOutput::markdown("reviewed"))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pause_during_fanout_stops_pending_item_launch() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let executor = WorkflowExecutor::new(store.clone(), WorkflowPolicy::default());
    let run = executor.start(fanout_spec()).unwrap();
    let run_id = run.id.clone();
    let launched = Arc::new(AtomicUsize::new(0));
    let runner = SlowFanoutRunner {
        launched: launched.clone(),
    };

    let task = tokio::spawn(async move {
        let _ = executor.execute_with_runner(run, &runner).await;
    });

    while launched.load(Ordering::SeqCst) < 2 {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    LifecycleController::new(store.clone())
        .apply(&run_id, LifecycleAction::Pause)
        .unwrap();
    task.await.unwrap();

    let paused = store.load_state(&run_id).unwrap();
    assert_eq!(paused.status, RunStatus::Paused);
    assert_eq!(
        paused.stages.get("review").unwrap().status,
        StageStatus::Paused
    );
    assert!(
        launched.load(Ordering::SeqCst) < 20,
        "pause should stop pending fanout items before all launch"
    );

    let resumed = LifecycleController::new(store)
        .apply(&run_id, LifecycleAction::Resume)
        .unwrap();
    assert_eq!(resumed.status, RunStatus::Running);
    assert_eq!(
        resumed.stages.get("review").unwrap().status,
        StageStatus::Pending
    );
}

#[test]
#[cfg(unix)]
fn cancel_during_command_stage_terminates_and_records_evidence() {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let spec = WorkflowSpec::from_yaml(&format!(
        r#"
schema: archon.workflow.v1
name: command-cancel
task: cancel local verification
target_repository_root: {}
stages:
  - id: verify
    kind: agent
    agent: local-verifier
    provider_tier: local
    verify_command: "sleep 30"
"#,
        repo.display()
    ))
    .unwrap();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let executor = WorkflowExecutor::new(store.clone(), WorkflowPolicy::default());
    let run = executor.start(spec).unwrap();
    let run_id = run.id.clone();
    let mut child = spawn_test_process(&repo);
    store
        .write_run_json(
            &run_id,
            "command-executions/verify/cmd-0001.json",
            &serde_json::json!({
                "schema": "archon.workflow.command_execution.v1",
                "run_id": run_id,
                "stage_id": "verify",
                "attempt_id": "verify-attempt",
                "command_id": "cmd-0001",
                "role": "verification",
                "command": "sleep 30",
                "cwd": repo.display().to_string(),
                "process_group": child.id(),
                "started_at": chrono::Utc::now().to_rfc3339(),
                "last_output_at": null,
                "last_progress_at": chrono::Utc::now().to_rfc3339(),
                "progress_class": "unknown_progress",
                "status": "running",
                "exit_status": null,
            }),
        )
        .unwrap();

    LifecycleController::new(store.clone())
        .apply(&run_id, LifecycleAction::Cancel)
        .unwrap();

    wait_for_child_exit(&mut child);

    let cancelled = store.load_state(&run_id).unwrap();
    assert_eq!(cancelled.status, RunStatus::Cancelled);
    assert_eq!(
        cancelled.stages.get("verify").unwrap().status,
        StageStatus::Cancelled
    );
    assert!(
        store
            .run_dir(&run_id)
            .join("command-cancellations")
            .exists(),
        "cancel should persist command cancellation evidence"
    );
}

#[cfg(unix)]
fn spawn_test_process(cwd: &std::path::Path) -> std::process::Child {
    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg("sleep 30")
        .current_dir(cwd)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    unsafe {
        use std::os::unix::process::CommandExt;
        command.pre_exec(|| {
            if libc::setpgid(0, 0) == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        });
    }
    command.spawn().unwrap()
}

#[cfg(unix)]
fn wait_for_child_exit(child: &mut std::process::Child) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if child.try_wait().unwrap().is_some() {
            return;
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            panic!("cancel should terminate the command-backed process group promptly");
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}
