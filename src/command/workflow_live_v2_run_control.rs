use super::*;

pub(super) fn finalize_generated_control(
    store: &WorkflowStore,
    run: &WorkflowRun,
    run_kind: archon_workflow::WorkflowRunKind,
    status: RunStatus,
    message: &str,
) -> archon_workflow::WorkflowResult<()> {
    // Lifecycle stops already persist their own generation. A resumed owner
    // must never be stopped again by the executor that is still unwinding.
    if store.load_state(&run.id)?.generation != run.generation {
        return Ok(());
    }
    match super::super::workflow_live_v2_finalizer::finalize_run_status(
        store,
        &run.id,
        run_kind,
        status,
        message,
        Some(run.generation),
    ) {
        // The owner may have changed after the read above; the finalizer
        // checks again under the lifecycle lock before any terminal write.
        Err(WorkflowError::ControlCancelled(_)) => Ok(()),
        result => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round3_generated_control_finalization_cannot_stop_a_resumed_run() {
        for status in [RunStatus::Paused, RunStatus::Cancelled] {
            let temp = tempfile::tempdir().unwrap();
            let store = WorkflowStore::project(temp.path());
            let run = store
                .create_run(super::super::super::workflow_run_finalizer_tests::spec())
                .unwrap();
            let lifecycle = LifecycleController::new(store.clone());
            lifecycle.apply(&run.id, LifecycleAction::Pause).unwrap();
            lifecycle.apply(&run.id, LifecycleAction::Resume).unwrap();
            let before = std::fs::read(store.state_path(&run.id)).unwrap();
            let events = std::fs::read(store.events_path(&run.id)).unwrap();
            finalize_generated_control(
                &store,
                &run,
                archon_workflow::WorkflowRunKind::FixedOrSavedScript,
                status,
                "obsolete executor stopped",
            )
            .unwrap();
            assert_eq!(
                store.load_state(&run.id).unwrap().status,
                RunStatus::Running
            );
            assert_eq!(std::fs::read(store.state_path(&run.id)).unwrap(), before);
            assert_eq!(std::fs::read(store.events_path(&run.id)).unwrap(), events);
            assert!(!store.run_dir(&run.id).join("v2/finalization.json").exists());
        }
    }
}
