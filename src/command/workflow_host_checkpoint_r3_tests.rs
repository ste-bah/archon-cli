use super::*;
struct ContendedProcess {
    records: std::path::PathBuf,
    mode: u32,
    calls: AtomicUsize,
}
#[async_trait::async_trait]
impl HostCommandProcessAdapter for ContendedProcess {
    async fn execute(
        &self,
        _request: ResolvedHostCommand,
        _control: HostCommandControl,
    ) -> WorkflowResult<SupervisedProcessOutput> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(
            crate::command::workflow_host_command_supervisor::checkpoint_contention(
                &self.records,
                self.mode,
            )
            .await,
        )
    }
}
async fn contention_pauses(mode: u32) {
    use crate::command::workflow_host_command_groups::{
        GROUP_RECORDS_DIR, require_no_running_groups,
    };
    let fixture = fixture(vec![]);
    let root = fixture.store.run_dir(&fixture.run_id);
    let process = Arc::new(ContendedProcess {
        records: root.join(GROUP_RECORDS_DIR),
        mode,
        calls: AtomicUsize::new(0),
    });
    let executor = FixedHostCommandExecutor::with_process(
        fixed_decomposition_catalog("rev-1").unwrap(),
        fixture.context.clone(),
        root.clone(),
        process.clone(),
    );
    let error = executor
        .execute(lint(), Some(fixture.generation))
        .await
        .unwrap_err();
    assert!(matches!(error, WorkflowError::ControlPaused(_)), "{error}");
    assert_eq!(
        fixture.store.load_state(&fixture.run_id).unwrap().status,
        RunStatus::Paused
    );
    assert_eq!(process.calls.load(Ordering::SeqCst), 1);
    assert!(error.to_string().contains(&format!(
        "archon workflow resume --live --yes {}",
        fixture.run_id
    )));
    assert!(
        require_no_running_groups(&root, &fixture.run_id).is_err(),
        "incomplete evidence must stay authoritative"
    );
}
#[tokio::test]
async fn initial_checkpoint_contention_pauses_executor() {
    contention_pauses(0).await;
}
#[tokio::test]
async fn scanner_checkpoint_contention_pauses_executor() {
    contention_pauses(1).await;
}
#[tokio::test]
async fn repeated_teardown_checkpoint_contention_pauses_executor() {
    contention_pauses(2).await;
}
