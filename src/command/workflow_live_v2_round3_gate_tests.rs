//! Admission order survives production completion and terminal unwind.
use super::*;

struct Gates {
    entered: tokio::sync::Notify,
}
#[async_trait::async_trait]
impl crate::command::workflow_host_command_exec::WorkflowHostCommandExecutor for Gates {
    fn call_identity(
        &self,
        request: &archon_workflow::HostCommandRequest,
    ) -> archon_workflow::WorkflowResult<String> {
        Ok(request.stdin.clone().unwrap())
    }
    fn record_is_reusable(
        &self,
        _: &WorkflowV2CallRecord,
    ) -> archon_workflow::WorkflowResult<bool> {
        Ok(false)
    }
    async fn execute(
        &self,
        request: archon_workflow::HostCommandRequest,
        _: Option<u64>,
    ) -> archon_workflow::WorkflowResult<archon_workflow::HostCommandResult> {
        if request.stdin.as_deref() == Some("candidate-a") {
            self.entered.notify_one();
            std::future::pending().await
        } else {
            Err(WorkflowError::StageFailed("candidate-b rejected".into()))
        }
    }
}
async fn gates(command: &str, subject: &str) {
    let (temp, store, id) = new_run();
    set_status(&store, &id, RunStatus::Running);
    crate::command::workflow_live::workflow_live_v2::workflow_live_v2_run::terminal_test_support::seed_fixed(&store, &id, temp.path());
    let executor = Arc::new(Gates {
        entered: tokio::sync::Notify::new(),
    });
    let (runner, _rx) = runner(
        &store,
        &id,
        Arc::new(PanicLlm),
        Some(executor.clone()),
        None,
    );
    runner
        .v2_store
        .bind_session_executor(store.load_state(&id).unwrap().generation);
    let host = host_of(runner);
    let payload = |id: &str, candidate: &str| {
        serde_json::json!({"id": id, "options": {"commandId": command, "stdin": candidate}})
            .to_string()
    };
    let first = host.execute("hostCommand".into(), payload("gate-a", "candidate-a"));
    tokio::pin!(first);
    tokio::select! { biased; result = &mut first => panic!("A must await its executor: {result:?}"), _ = executor.entered.notified() => {} }
    let admitted = host
        .runner
        .v2_store
        .load_call_records()
        .unwrap()
        .into_iter()
        .find(|r| r.status == WorkflowV2Status::Running)
        .expect("A admitted before executor entry");
    let first_id = admitted.call.id.clone();
    assert_eq!(admitted.status, WorkflowV2Status::Running);
    let second = host
        .execute("hostCommand".into(), payload("gate-b", "candidate-b"))
        .await;
    assert!(
        second.is_ok(),
        "B returns its failure to the driver: {second:?}"
    );
    let stop = host
        .execute(
            "humanGate".into(),
            serde_json::json!({"id":"terminal", "options":{"task":"Require approval"}}).to_string(),
        )
        .await;
    assert!(
        matches!(stop, Err(WorkflowError::TerminalHostCall(_))),
        "terminal gate unwinds pending A: {stop:?}"
    );
    host.interrupt_terminal_calls().await;
    let first = host
        .runner
        .v2_store
        .load_call_record(&first_id)
        .unwrap()
        .unwrap();
    let second = host
        .runner
        .v2_store
        .load_call_records()
        .unwrap()
        .into_iter()
        .find(|r| r.call.id != first_id && r.call.method == WorkflowV2HostMethod::HostCommand)
        .unwrap();
    assert_ne!(
        first.call.id, second.call.id,
        "distinct concurrent gate IDs"
    );
    assert_eq!(first.result.data["interrupted"], "terminal_host_stop");
    assert_eq!(second.status, WorkflowV2Status::Failed);
    let state: archon_workflow::FixedDecompositionStateV1 = serde_json::from_slice(
        &std::fs::read(
            store
                .run_dir(&id)
                .join(crate::command::workflow_decompose_state::FIXED_STATE_PATH),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        state.dispositions[subject],
        archon_workflow::SubjectDisposition::Failed,
        "A's later unwind must not supersede B's failure"
    );
    assert_eq!(
        first.started_at, admitted.started_at,
        "admission time survives unwind"
    );
    assert_eq!(first.admission_sequence, admitted.admission_sequence);
    assert!(first.admission_sequence < second.admission_sequence);
    assert!(
        first.started_at < second.started_at,
        "distinct calls keep their real admission order"
    );
}
#[tokio::test]
async fn round3_291_acceptance_gates_terminal_unwind() {
    gates("freeze-acceptance", "freeze-acceptance").await;
}
#[tokio::test]
async fn round3_291_skeleton_gates_terminal_unwind() {
    gates("freeze-skeleton", "freeze-skeleton").await;
}
#[tokio::test]
async fn round3_291_set_gates_terminal_unwind() {
    gates("task-set-lint", "task-set-lint").await;
}
