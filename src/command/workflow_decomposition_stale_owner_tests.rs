//! Issue 251: a fixed run left `Running` by a killed executor resumes with a
//! recorded `stale_owner_recovered` event, while a live executor still blocks
//! a second resume.

use super::*;

const HOLD_RUN_ENV: &str = "ARCHON_TEST_HOLD_EXECUTOR_LEASE";
const HOLD_READY_ENV: &str = "ARCHON_TEST_HOLD_EXECUTOR_LEASE_READY";

/// Runs one launch with this binary, then leaves the run `Running`, as a
/// killed executor does.
async fn launch_left_running(project: &Path) -> (WorkflowStore, String) {
    let root = project
        .canonicalize()
        .map(archon_shell::paths::plain)
        .unwrap();
    let _ = run_fixed_decomposition_with_factory(
        project,
        Path::new("prds/PRD-X.md"),
        Path::new("tasks/PRD-X"),
        None,
        true,
        &launch_config(project),
        &empty_env(),
        &BarrierFactory::launch(root.clone()),
    )
    .await;
    let store = WorkflowStore::project(&root);
    let run_id = store.list_runs().unwrap().pop().unwrap().id;
    store
        .with_run_lock(&run_id, |locked| {
            let mut run = locked.load_state(&run_id)?;
            run.status = RunStatus::Running;
            run.generation += 1;
            locked.save_state(&run)
        })
        .unwrap();
    (store, run_id)
}

async fn resume(project: &Path) -> (BarrierFactory, String) {
    let root = project
        .canonicalize()
        .map(archon_shell::paths::plain)
        .unwrap();
    let run_id = WorkflowStore::project(&root)
        .list_runs()
        .unwrap()
        .pop()
        .unwrap()
        .id;
    let factory = BarrierFactory::resume(root);
    let error = resume_fixed_decomposition_with_factory(
        project,
        &run_id,
        true,
        &launch_config(project),
        &empty_env(),
        &factory,
    )
    .await
    .unwrap_err();
    (factory, format!("{error:#}"))
}

fn recovery_events(store: &WorkflowStore, run_id: &str) -> Vec<serde_json::Value> {
    let events = std::fs::read_to_string(store.events_path(run_id)).unwrap_or_default();
    events
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .filter(|event| event["kind"] == "stale_owner_recovered")
        .collect()
}

/// The child half of the dead-owner test: takes the run's executor lease
/// through the production path, says so, and holds it until it is killed.
#[test]
#[ignore = "child process of fixed_resume_recovers_a_killed_owner_and_refuses_a_live_one"]
fn hold_executor_lease_until_killed() {
    let (Ok(target), Ok(ready)) = (std::env::var(HOLD_RUN_ENV), std::env::var(HOLD_READY_ENV))
    else {
        return;
    };
    let (project_root, run_id) = target.split_once('\n').unwrap();
    let store = WorkflowStore::project(Path::new(project_root));
    let _lease =
        crate::command::workflow_task_root_reclaim::begin_execution(&store, run_id).unwrap();
    std::fs::write(&ready, "held").unwrap();
    std::thread::sleep(std::time::Duration::from_secs(300));
}

/// Kills the lease holder however the test ends; it is this test's own child.
struct LeaseHolderChild(std::process::Child);

impl LeaseHolderChild {
    fn kill(&mut self) {
        self.0.kill().unwrap();
        self.0.wait().unwrap();
    }
}

impl Drop for LeaseHolderChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn spawn_lease_holder(project_root: &Path, run_id: &str, ready: &Path) -> LeaseHolderChild {
    let module = module_path!();
    let module = module.split_once("::").map_or(module, |(_, rest)| rest);
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            &format!("{module}::hold_executor_lease_until_killed"),
            "--exact",
            "--ignored",
            "--test-threads=1",
        ])
        .env(
            HOLD_RUN_ENV,
            format!("{}\n{run_id}", project_root.to_string_lossy()),
        )
        .env(HOLD_READY_ENV, ready)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    while !ready.exists() {
        if let Some(status) = child.try_wait().unwrap() {
            panic!("lease holder exited before taking the lease: {status}");
        }
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("lease holder did not take the lease in time");
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    LeaseHolderChild(child)
}

#[tokio::test]
async fn fixed_resume_recovers_a_killed_owner_and_refuses_a_live_one() {
    let project = fixture_project();
    let (store, run_id) = launch_left_running(project.path()).await;
    let signals = tempfile::tempdir().unwrap();
    let ready = signals.path().join("lease-holder.ready");
    let root = project
        .path()
        .canonicalize()
        .map(archon_shell::paths::plain)
        .unwrap();
    let mut holder = spawn_lease_holder(&root, &run_id, &ready);
    let holder_pid = holder.0.id();

    // Live owner: refused, the pid is named, and nothing changes.
    let (factory, live_error) = resume(project.path()).await;
    assert!(live_error.contains("is live"), "{live_error}");
    #[cfg(unix)]
    assert!(
        live_error.contains(&format!("process {holder_pid} holds")),
        "{live_error}"
    );
    assert_eq!(factory.builds.load(Ordering::SeqCst), 0);
    assert_eq!(
        store.load_state(&run_id).unwrap().status,
        RunStatus::Running
    );
    assert!(recovery_events(&store, &run_id).is_empty());
    holder.kill();

    // Dead owner: the kernel released the lock; resume records the
    // recovery, pauses, and proceeds to provider construction.
    let (factory, error) = resume(project.path()).await;
    assert!(error.contains("barrier observed"), "{error}");
    assert_eq!(factory.builds.load(Ordering::SeqCst), 1);
    assert_eq!(store.load_state(&run_id).unwrap().status, RunStatus::Paused);
    let events = recovery_events(&store, &run_id);
    assert_eq!(events.len(), 1, "{events:?}");
    let detail = &events[0]["detail"];
    assert_eq!(detail["previous_status"], "running");
    assert_eq!(detail["status"], "paused");
    assert_eq!(detail["owner_lock_free"], true);
    assert_eq!(detail["previous_owner_pid"], holder_pid);
    assert_eq!(detail["recovered_by_pid"], std::process::id());
    #[cfg(unix)]
    assert_eq!(detail["previous_owner_pid_running"], false);

    let state: FixedDecompositionStateV1 =
        read_json(&store.run_dir(&run_id).join(FIXED_DECOMPOSITION_STATE_PATH));
    let log = std::fs::read_to_string(&state.log_path).unwrap();
    assert!(
        log.contains(&format!(
            "transition=stale_owner_recovered previous_pid={holder_pid}"
        )),
        "{log}"
    );
}

#[tokio::test]
async fn fixed_resume_treats_a_reused_owner_pid_as_dead_when_the_lock_is_free() {
    let project = fixture_project();
    let (store, run_id) = launch_left_running(project.path()).await;
    // The record names a pid that is alive (this test process) but does not
    // hold the lock: the OS gave the dead owner's pid to another process.
    let reused = std::process::id();
    std::fs::write(
        store.run_dir(&run_id).join("decomposition/executor.lock"),
        serde_json::to_vec(&serde_json::json!({
            "schema_version": 1, "pid": reused, "acquired_at": "2026-10-03T00:00:00Z"
        }))
        .unwrap(),
    )
    .unwrap();
    let inflight = store.run_dir(&run_id).join("v2/inflight");
    std::fs::create_dir_all(&inflight).unwrap();
    std::fs::write(inflight.join("a.json"), r#"{"host_pid":4242}"#).unwrap();
    std::fs::write(inflight.join("b.json"), r#"{"host_pid":4242}"#).unwrap();

    let (factory, error) = resume(project.path()).await;

    assert!(error.contains("barrier observed"), "{error}");
    assert_eq!(factory.builds.load(Ordering::SeqCst), 1);
    let events = recovery_events(&store, &run_id);
    assert_eq!(events.len(), 1, "{events:?}");
    let detail = &events[0]["detail"];
    assert_eq!(detail["previous_owner_pid"], reused);
    assert_eq!(detail["previous_owner_acquired_at"], "2026-10-03T00:00:00Z");
    #[cfg(unix)]
    assert_eq!(detail["previous_owner_pid_running"], true);
    assert_eq!(detail["orphaned_inflight_markers"], 2);
    assert_eq!(detail["inflight_host_pids"], serde_json::json!([4242]));
}

#[tokio::test]
async fn fixed_resume_of_a_paused_run_records_no_recovery() {
    let project = fixture_project();
    let (store, run_id) = launch_left_running(project.path()).await;
    archon_workflow::LifecycleController::new(store.clone())
        .apply(&run_id, archon_workflow::LifecycleAction::Pause)
        .unwrap();

    let (_, error) = resume(project.path()).await;

    assert!(error.contains("barrier observed"), "{error}");
    assert!(recovery_events(&store, &run_id).is_empty());
    // The lease file now names the holder that took it.
    let record: serde_json::Value =
        read_json(&store.run_dir(&run_id).join("decomposition/executor.lock"));
    assert_eq!(record["pid"], std::process::id());
}
