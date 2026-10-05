//! Issue 329 resume compatibility: a fixed decomposition paused before this
//! binary resumes on it. The launch now checks executor ownership and binds
//! the session to the resume generation under the run lock. A state written
//! before `executor_generation` existed, and one written with it, both
//! resume through the real launch, reuse every recorded attempt and complete.
use super::*;
use crate::command::workflow_live::WorkflowScriptPlan;
use crate::command::workflow_live::workflow_live_v2::workflow_live_v2_run::terminal_test_support::{
    save_fixed_metadata, seed_fixed,
};

/// Launches (or, after a resume, relaunches) the fixed decomposition through
/// the same entry `workflow decompose` and its resume use.
async fn launch(
    temp: &tempfile::TempDir,
    store: &WorkflowStore,
    run: archon_workflow::WorkflowRun,
    llm: &Arc<CountingLlm>,
    host: &Arc<StallingHost>,
) -> String {
    let mut plan = WorkflowScriptPlan::from_template(
        run.spec.clone(),
        crate::command::workflow_decompose::FIXED_SCRIPT_SOURCE,
        Vec::new(),
    );
    plan.script_args = Some(fixed_args(temp.path()));
    save_fixed_metadata(store, &run.id, &plan);
    let (ui, _rx) = default_workflow_ui_sink();
    crate::command::workflow_live::execute_fixed_decomposition_v2_run(
        store,
        run,
        plan,
        llm.clone(),
        ui,
        Vec::new(),
        host.clone(),
    )
    .await
    .unwrap()
}

/// The state as a binary from before `executor_generation` wrote it.
fn drop_executor_generation(store: &WorkflowStore, run_id: &str) {
    let path = store.state_path(run_id);
    let mut state: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    state.as_object_mut().unwrap().remove("executor_generation");
    std::fs::write(&path, serde_json::to_vec_pretty(&state).unwrap()).unwrap();
    assert_eq!(store.load_state(run_id).unwrap().executor_generation, None);
}

#[tokio::test]
async fn a_fixed_decomposition_paused_before_this_binary_resumes_and_completes() {
    for written_before_the_field in [false, true] {
        let (temp, store, run_id, llm, host) = fixture();
        seed_fixed(&store, &run_id, temp.path());
        let first = launch(
            &temp,
            &store,
            store.load_state(&run_id).unwrap(),
            &llm,
            &host,
        )
        .await;
        assert!(first.starts_with("Fixed decomposition paused"), "{first}");
        assert_eq!(llm.calls.load(Ordering::SeqCst), 6, "{first}");
        if written_before_the_field {
            drop_executor_generation(&store, &run_id);
        }

        host.fixed.store(true, Ordering::SeqCst);
        // What `workflow decompose resume` does before it launches.
        let resumed = archon_workflow::LifecycleController::new(store.clone())
            .apply(&run_id, archon_workflow::LifecycleAction::Resume)
            .unwrap();
        let report = launch(&temp, &store, resumed, &llm, &host).await;

        assert!(
            report.contains("status Accepted"),
            "before the field: {written_before_the_field}: {report}"
        );
        assert_eq!(
            llm.calls.load(Ordering::SeqCst),
            7,
            "every recorded author call is reused; one new attempt follows the pause"
        );
        assert_eq!(pause_events(&store, &run_id).len(), 1);
        assert_eq!(
            store.load_state(&run_id).unwrap().status,
            archon_workflow::RunStatus::Completed,
            "{report}"
        );
    }
}
