//! Issue 261: a pause request belongs to the executor that makes it. Once a
//! resume hands the run to a newer executor generation, an older executor
//! still running its script cannot pause the run: it is refused as stale, and
//! the run, its generation and its event log stay as the new owner left them.
use super::*;

#[tokio::test]
async fn a_stale_executor_cannot_pause_a_run_a_newer_generation_owns() {
    let (_temp, store, run_id) = new_run();
    set_status(&store, &run_id, archon_workflow::RunStatus::Running);
    let bound = store.load_state(&run_id).unwrap().generation;
    let (mut runner, _rx) = runner(&store, &run_id, Arc::new(PanicLlm), None, None);
    // What `run` binds when the executor starts under `bound`.
    runner.start_generation = Some(bound);
    let host = WorkflowScriptHost {
        scaffold_hash: String::new(),
        host_occurrences: Default::default(),
        envelope_shape: ScriptEnvelopeShape::Compat,
        runner,
        accumulator: Arc::new(tokio::sync::Mutex::new(Default::default())),
        tool_host: Default::default(),
        tool_budget: Arc::new(std::sync::Mutex::new(Default::default())),
    };
    // An operator pause and resume hand the run to a newer executor.
    archon_workflow::LifecycleController::new(store.clone())
        .apply(&run_id, archon_workflow::LifecycleAction::Pause)
        .unwrap();
    resume(&store, &run_id);
    let owner = store.load_state(&run_id).unwrap();
    assert_eq!(owner.status, archon_workflow::RunStatus::Running);
    assert!(
        owner.executor_generation.is_some_and(|g| g > bound),
        "the resume took the run over: {:?}",
        owner.executor_generation
    );

    let outcome = host
        .execute(
            archon_workflow::v2::script::SCRIPT_PAUSE_METHOD.into(),
            serde_json::json!({
                "id": "pause-stale-1",
                "options": { "evidence": { "subject": "stale", "reason": "no_progress" } },
            })
            .to_string(),
        )
        .await;

    let after = store.load_state(&run_id).unwrap();
    assert_eq!(
        (after.status.clone(), after.generation),
        (archon_workflow::RunStatus::Running, owner.generation),
        "a stale executor paused the run generation {} owns",
        owner.generation
    );
    assert!(
        matches!(&outcome, Err(WorkflowError::ControlCancelled(message)) if message.contains("no longer owns")),
        "the stale caller is refused, never answered: {outcome:?}"
    );
    assert!(pause_events(&store, &run_id).is_empty());
    assert!(
        !store.run_dir(&run_id).join("v2/script-pauses").exists(),
        "a refused request records no pause"
    );
}
