//! Round 4 (Issue 291): repository-audit dispatch uses the one bound-store
//! fence. A stale dispatch after its successor removed the run refuses typed,
//! dispatches nothing and never recreates the run directory.
use super::*;
use archon_workflow::WorkflowAgentDispatch;

fn names(store: &WorkflowStore) -> std::collections::BTreeSet<std::ffi::OsString> {
    std::fs::read_dir(store.root())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect()
}

async fn removed_run_audit(actions: &[LifecycleAction]) {
    let (_temp, store, id) = new_run();
    set_status(&store, &id, RunStatus::Running);
    let llm = Arc::new(CountingAcceptedLlm {
        calls: AtomicUsize::new(0),
    });
    let (runner, _rx) = runner(&store, &id, llm.clone(), None, None);
    runner
        .v2_store
        .bind_session_executor(store.load_state(&id).unwrap().generation);
    let ctl = LifecycleController::new(store.clone());
    for action in actions {
        ctl.apply(&id, action.clone()).unwrap();
    }
    std::fs::remove_dir_all(store.run_dir(&id)).unwrap();
    let before = names(&store);
    let dispatch = AuditDispatch(runner.client.for_audit());
    let execution = WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            id: "audit-after-removal".into(),
            method: WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: Default::default(),
        },
        input: serde_json::json!({}),
        depends_on: vec![],
    };
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        dispatch.run_call(
            "assess",
            None,
            &execution,
            &WorkflowV2AgentAdapter::new(),
            Some(&runner.v2_store),
            None,
        ),
    )
    .await
    .expect("a refused audit ends at once");
    assert!(
        matches!(&result, Err(WorkflowError::ControlCancelled(message)) if message.contains("no longer exists")),
        "{actions:?}: a typed stop, never a stage failure: {result:?}"
    );
    assert_eq!(llm.calls.load(Ordering::SeqCst), 0, "nothing dispatched");
    assert!(
        !store.run_dir(&id).exists(),
        "{actions:?}: the removed run directory is not recreated"
    );
    assert_eq!(names(&store), before, "no run namespace appears");
}

#[tokio::test]
async fn round4_291_audit_dispatch_after_removed_paused_run() {
    removed_run_audit(&[LifecycleAction::Pause]).await;
}
#[tokio::test]
async fn round4_291_audit_dispatch_after_removed_resumed_run() {
    removed_run_audit(&[LifecycleAction::Pause, LifecycleAction::Resume]).await;
}
#[tokio::test]
async fn round4_291_audit_dispatch_after_removed_cancelled_run() {
    removed_run_audit(&[LifecycleAction::Cancel]).await;
}
#[tokio::test]
async fn round4_291_audit_dispatch_after_removed_running_run() {
    removed_run_audit(&[]).await;
}
