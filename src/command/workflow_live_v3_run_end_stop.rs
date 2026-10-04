use super::*;

pub(in super::super) fn stop(
    store: &WorkflowStore,
    run_id: &str,
    run_kind: WorkflowRunKind,
    status: archon_workflow::RunStatus,
    message: &str,
    expected_generation: Option<u64>,
) -> WorkflowResult<()> {
    stop_with(store, run_id, expected_generation, || {
        super::super::workflow_live_v2_finalizer::finalize_run_status(
            store,
            run_id,
            run_kind,
            status,
            message,
            expected_generation,
        )
    })
}

// Keep the early ownership check and the locked commit separately exercisable.
fn stop_with(
    store: &WorkflowStore,
    run_id: &str,
    expected_generation: Option<u64>,
    finalize: impl FnOnce() -> WorkflowResult<()>,
) -> WorkflowResult<()> {
    if expected_generation.is_some_and(|expected| {
        store
            .load_state(run_id)
            .is_ok_and(|run| !run.execution_owned_at(expected))
    }) {
        return Ok(());
    }
    match finalize() {
        // Ownership may change between the early read and the locked write.
        Err(archon_workflow::WorkflowError::ControlCancelled(_)) => Ok(()),
        result => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_workflow::{LifecycleAction, LifecycleController, RunStatus};

    #[test]
    fn round4_losing_ownership_at_the_locked_stop_is_a_noop() {
        for status in [RunStatus::Paused, RunStatus::Cancelled] {
            let temp = tempfile::tempdir().unwrap();
            let store = WorkflowStore::project(temp.path());
            let run = store
                .create_run(super::super::super::workflow_run_finalizer_tests::spec())
                .unwrap();
            let result = stop_with(&store, &run.id, Some(run.generation), || {
                // The real owner changes after the early check but before the
                // finalizer's locked check. No timing or sleeps are needed.
                let lifecycle = LifecycleController::new(store.clone());
                lifecycle.apply(&run.id, LifecycleAction::Pause)?;
                lifecycle.apply(&run.id, LifecycleAction::Resume)?;
                super::super::super::workflow_live_v2_finalizer::finalize_run_status(
                    &store,
                    &run.id,
                    WorkflowRunKind::FixedOrSavedScript,
                    status.clone(),
                    "obsolete stop",
                    Some(run.generation),
                )
            });
            assert!(result.is_ok(), "{status:?}: {result:?}");
            assert_eq!(
                store.load_state(&run.id).unwrap().status,
                RunStatus::Running
            );
            assert!(!store.run_dir(&run.id).join("v2/finalization.json").exists());
            assert!(
                !std::fs::read_to_string(store.events_path(&run.id))
                    .unwrap()
                    .contains("terminal_status")
            );
        }
    }
}
