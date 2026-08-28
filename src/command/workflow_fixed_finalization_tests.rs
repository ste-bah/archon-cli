use super::*;

use archon_workflow::{WorkflowRunKind, WorkflowV2ResultStore, WorkflowV2Status};

fn summary() -> workflow_live_v2_script::WorkflowV2ScriptSummary {
    workflow_live_v2_script::WorkflowV2ScriptSummary {
        status: WorkflowV2Status::Accepted,
        completed: 0,
        executed: 0,
        reused: 0,
        calls: Vec::new(),
        failed_call: None,
        failed_result_path: None,
        next_action: None,
        script_result: None,
    }
}

fn spec() -> archon_workflow::WorkflowSpec {
    archon_workflow::WorkflowSpec {
        schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
        name: "fixed-finalization-owner".into(),
        task: "test fixed finalization ownership".into(),
        target_repository_root: None,
        max_parallelism: 1,
        max_agents: 1,
        stages: Vec::new(),
        permissions: Default::default(),
        learning_hooks: Vec::new(),
    }
}

#[test]
fn fixed_pause_and_cancel_branches_never_write_terminal_finalization() {
    let source = include_str!("workflow_live_v2_fixed_run.rs");
    for (start, end) in [
        (
            "Err(WorkflowError::ControlPaused",
            "Err(WorkflowError::ControlCancelled",
        ),
        ("Err(WorkflowError::ControlCancelled", "Err(error)"),
    ] {
        let body = &source[source.find(start).unwrap()..source.find(end).unwrap()];
        assert!(!body.contains("finalize_run_status"), "{body}");
        assert!(!body.contains("finalize_summary"), "{body}");
    }
}

#[tokio::test]
async fn obsolete_fixed_generation_cannot_persist_terminal_state_or_record() {
    let temp = tempfile::tempdir().unwrap();
    let store = archon_workflow::WorkflowStore::project(temp.path());
    let run = store.create_run(spec()).unwrap();
    let v2 = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    let old_generation = run.generation;
    let lifecycle = archon_workflow::LifecycleController::new(store.clone());
    lifecycle
        .apply(&run.id, archon_workflow::LifecycleAction::Pause)
        .unwrap();
    lifecycle
        .apply(&run.id, archon_workflow::LifecycleAction::Resume)
        .unwrap();

    let error = workflow_live_v2_finalizer::finalize_summary(
        &store,
        &run.id,
        WorkflowRunKind::FixedDecompositionV1,
        None,
        &summary(),
        &v2,
        None,
        Some(old_generation),
    )
    .await
    .unwrap_err();

    assert!(matches!(
        error,
        archon_workflow::WorkflowError::ControlCancelled(_)
    ));
    assert_eq!(
        store.load_state(&run.id).unwrap().status,
        archon_workflow::RunStatus::Running
    );
    assert!(
        !store
            .run_dir(&run.id)
            .join(workflow_live_v2_finalizer::FINALIZATION_RECORD_PATH)
            .exists()
    );
}
