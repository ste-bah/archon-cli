use super::*;
struct ReadThenChild(Arc<RecoveringProcess>);
#[async_trait::async_trait]
impl HostCommandProcessAdapter for ReadThenChild {
    async fn execute(
        &self,
        _: crate::command::workflow_host_command_catalog::ResolvedHostCommand,
        _: HostCommandControl,
    ) -> archon_workflow::WorkflowResult<SupervisedProcessOutput> {
        self.0.calls.fetch_add(1, Ordering::SeqCst);
        Err(WorkflowError::StageFailed("child execution reached".into()))
    }
}
#[tokio::test]
async fn host_command_launch_input_transient_read_retries_before_child_execution() {
    let mut fixture = fixture(0, false);
    let path = fixture.executor.context_for_test().prd_path.clone();
    let fault = inject(
        Point::LaunchInput,
        &path,
        Box::new(|| Err(std::io::ErrorKind::Interrupted.into())),
    );
    let process = Arc::new(ReadThenChild(fixture.process.clone()));
    Arc::get_mut(&mut fixture.executor)
        .unwrap()
        .replace_process_for_test(process);
    run_script(&fixture, r#"async function workflow(w) { return await w.hostCommand("freeze-acceptance", { stdin: "{}" }); }"#).await.expect("the input read recovers before dispatch");
    assert!(fault.fired());
    assert_eq!(fixture.process.calls.load(Ordering::SeqCst), 1);
    let recorded = records(&fixture);
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].status, WorkflowV2Status::Failed);
    assert!(
        recorded[0]
            .result
            .summary
            .contains("child execution reached"),
        "{:?}",
        recorded[0].result
    );
}
