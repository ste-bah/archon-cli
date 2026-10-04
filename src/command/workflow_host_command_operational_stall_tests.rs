//! Issue 270 round 3: a stalled teardown pauses the run at once. No
//! operational retry runs while a kept record's survivors still run, and a
//! resume refuses until they are gone.
use super::*;
use crate::command::workflow_host_command_groups::{
    GROUP_RECORDS_DIR, record_group, require_no_running_groups,
};

/// The first attempt leaves a survivor its teardown could not kill: it keeps
/// a stalled record naming the survivor, and ends with the resumable exit.
/// Any later attempt would succeed, which a retry must never reach.
struct StallingProcess {
    calls: AtomicUsize,
    records: std::path::PathBuf,
    survivor: std::sync::Mutex<Option<std::process::Child>>,
}

#[async_trait::async_trait]
impl HostCommandProcessAdapter for StallingProcess {
    async fn execute(
        &self,
        _request: ResolvedHostCommand,
        _control: HostCommandControl,
    ) -> WorkflowResult<SupervisedProcessOutput> {
        if self.calls.fetch_add(1, Ordering::SeqCst) > 0 {
            return Ok(output(Some(0), false, ""));
        }
        let child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let pid = child.id();
        let start = archon_shell::process_tree::start_of(pid).unwrap();
        // A group id no live process carries, as after the leader's exit.
        let ended = std::process::Command::new("true").spawn().unwrap();
        let ended_pgid = ended.id();
        drop(ended.wait_with_output());
        record_group(
            &self.records,
            ended_pgid,
            ended_pgid,
            None,
            None,
            "task-set-lint",
        )
        .unwrap()
        .keep(Some(&[(pid, start)]));
        *self.survivor.lock().unwrap() = Some(child);
        Ok(output(
            Some(EXIT_INCOMPLETE_RESUMABLE),
            false,
            "host command teardown stalled: a member would not die",
        ))
    }
}

#[tokio::test]
async fn a_stalled_teardown_pauses_the_run_without_a_retry_and_resume_refuses() {
    let temp = tempfile::tempdir().unwrap();
    let mut context = context(temp.path());
    let store = WorkflowStore::project(&context.project_root);
    let mut run = store
        .create_run(archon_workflow::WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
            name: "host-stall".into(),
            task: "test".into(),
            target_repository_root: None,
            max_parallelism: 1,
            max_agents: 1,
            stages: Vec::new(),
            permissions: Default::default(),
            learning_hooks: Vec::new(),
        })
        .unwrap();
    run.status = RunStatus::Running;
    let mut stage = archon_workflow::run::StageState::pending("host-call");
    stage.status = StageStatus::Running;
    run.stages.insert("host-call".into(), stage);
    store.save_state(&run).unwrap();
    let run_root = store.run_dir(&run.id);
    context.run_staging_root = run_root.join("host-command-staging");
    let process = Arc::new(StallingProcess {
        calls: AtomicUsize::new(0),
        records: run_root.join(GROUP_RECORDS_DIR),
        survivor: std::sync::Mutex::new(None),
    });
    let executor = FixedHostCommandExecutor::with_process(
        fixed_decomposition_catalog("rev-1").unwrap(),
        context,
        run_root.clone(),
        process.clone(),
    );

    let error = executor
        .execute(lint(), Some(run.generation))
        .await
        .unwrap_err();
    let refused = require_no_running_groups(&run_root, &run.id);
    let mut survivor = process.survivor.lock().unwrap().take().unwrap();
    let survivor_pid = survivor.id();
    let _ = survivor.kill();
    let _ = survivor.wait();

    assert!(matches!(error, WorkflowError::ControlPaused(_)), "{error}");
    assert_eq!(
        process.calls.load(Ordering::SeqCst),
        1,
        "no retry may run while the stalled teardown's survivor runs"
    );
    assert_eq!(store.load_state(&run.id).unwrap().status, RunStatus::Paused);
    let lines: Vec<serde_json::Value> = std::fs::read_to_string(store.events_path(&run.id))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let stalls: Vec<_> = lines
        .iter()
        .filter(|e| e["detail"]["event"] == "host_command_teardown_stalled")
        .collect();
    assert_eq!(stalls.len(), 1, "the stall is recorded with its evidence");
    assert!(
        stalls[0]["detail"]["evidence"]
            .as_str()
            .unwrap()
            .contains(&survivor_pid.to_string())
    );
    let pause = lines
        .iter()
        .find(|e| e["detail"]["event"] == "host_command_operational_pause")
        .expect("the run is paused through the operational path");
    assert_eq!(pause["detail"]["cause"], "teardown_stalled");
    let refusal = refused.expect_err("a resume refuses while the survivor runs");
    assert!(
        refusal.to_string().contains(&survivor_pid.to_string()),
        "{refusal}"
    );
    // The survivor gone, the record ends and a resume may go ahead.
    let start = std::time::Instant::now();
    while require_no_running_groups(&run_root, &run.id).is_err() {
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}
