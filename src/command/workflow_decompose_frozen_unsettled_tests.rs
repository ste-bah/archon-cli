use super::*;

struct InterruptedProcess {
    local: LocalProcess,
    journal: PathBuf,
    bytes: Vec<u8>,
    log: PathBuf,
}

#[async_trait::async_trait]
impl crate::command::workflow_host_command_exec::HostCommandProcessAdapter for InterruptedProcess {
    async fn execute(
        &self,
        request: crate::command::workflow_host_command_catalog::ResolvedHostCommand,
        control: crate::command::workflow_host_command_supervisor::HostCommandControl,
    ) -> archon_workflow::WorkflowResult<
        crate::command::workflow_host_command_supervisor::SupervisedProcessOutput,
    > {
        // Interrupt after admission has read the chain and before the real child reads it.
        std::fs::write(&self.journal, &self.bytes).unwrap();
        std::fs::create_dir_all(&self.log).unwrap();
        self.local.execute(request, control).await
    }
}

#[tokio::test]
async fn verify_frozen_unsettled_publish_pauses_through_real_parent() {
    tokio::time::timeout(std::time::Duration::from_secs(60), async {
        for stage in ["acceptance", "skeleton"] {
            let mut f = project();
            freeze_acceptance(&mut f);
            let pin = crate::command::workflow_task_set::acceptance_pin_path(&f.project, &f.tasks);
            freeze_skeleton(&f);
            let [journal, _] = archon_workflow::task_set_publish_lock::journal_paths(&pin);
            let log = archon_workflow::task_set_publish_lock::recovery_log_path(&pin);
            let bytes = serde_json::to_vec(&serde_json::json!({
                "schema_version": 1, "transaction": "a".repeat(32),
                "state": "committed", "remedy": "restore writable recovery evidence", "entries": []
            })).unwrap();
            let store = WorkflowStore::project(&f.project);
            let run = run(&store);
            let mut context = crate::command::workflow_host_command_exec_tests::context(f._temp.path());
            context.project_root = f.project.clone();
            context.task_root = f.tasks.clone();
            context.prd_path = f.prd.clone();
            context.prd_digest = archon_workflow::task_set_contract::content_digest(&std::fs::read(&f.prd).unwrap());
            context.program = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().join("archon");
            let executor = FixedHostCommandExecutor::with_process(
                crate::command::workflow_host_command_catalog::fixed_decomposition_catalog("fixture").unwrap(),
                context, store.run_dir(&run.id), std::sync::Arc::new(InterruptedProcess { local: LocalProcess(f.project.clone()), journal, bytes, log: log.clone() }),
            );
            let result = executor.execute(
                HostCommandRequest::new(&format!("verify-frozen-{stage}"), None).unwrap(),
                Some(run.generation),
            ).await;
            assert!(matches!(&result, Err(archon_workflow::WorkflowError::ControlPaused(text)) if text.contains("state: committed") && text.contains("Operator remedy")), "{stage}: {result:?}");
            assert_eq!(store.load_state(&run.id).unwrap().status, archon_workflow::RunStatus::Paused);
            let events = std::fs::read_to_string(store.events_path(&run.id)).unwrap();
            let paused = events.lines().map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
                .filter(|event| event["detail"]["event"] == "host_command_unsettled_publish").count();
            assert_eq!(paused, 1, "{events}");
            assert!(!events.contains("host_command_operational_retry"), "{events}");
            std::fs::remove_dir(log).unwrap();
        }
    }).await.expect("test watchdog");
}
