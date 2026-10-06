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

/// Issue 316: a control cancellation of a run nobody cancelled, while the
/// launch executor still owns it, is a pause that was refused. Every run-end
/// pause is checked against the executor with the pause under the run lock,
/// so this must not happen; should it, the run is never written Cancelled.
/// It is named, logged as an error, and paused for the executor. `None` for
/// a real cancellation, an unfenced finalization or a replaced executor.
pub(in super::super) fn refused_while_owned(
    store: &WorkflowStore,
    run_id: &str,
    expected_generation: Option<u64>,
    message: &str,
) -> Option<archon_workflow::WorkflowError> {
    let launch = expected_generation?;
    let run = store.load_state(run_id).ok()?;
    if run.status == archon_workflow::RunStatus::Cancelled || !run.execution_owned_at(launch) {
        return None;
    }
    let reason = format!(
        "a run-end pause was refused while executor {launch} still owns run {run_id} ({message}); this must not happen"
    );
    tracing::error!(run_id, launch, %message, "{reason}; the run is paused, never Cancelled");
    let resume = format!("archon workflow resume --live --yes {run_id}");
    let detail = serde_json::json!({
        "event": "run_end_refused_pause", "reason": reason, "resume": resume,
    });
    let owner = archon_workflow::control_pause::PauseOwner::Executor(launch);
    Some(
        match super::owned_pause::pause(store, run_id, owner, detail) {
            Ok(_) => archon_workflow::WorkflowError::ControlPaused(format!(
                "{reason}; the run is paused, not cancelled: {resume}"
            )),
            Err(error) => error,
        },
    )
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
        // Issue 291: a stale executor's stop is refused, never silently.
        tracing::warn!(
            run_id,
            ?expected_generation,
            "stale executor stop refused: a newer executor owns the run"
        );
        return Ok(());
    }
    match finalize() {
        // Ownership may change between the early read and the locked write.
        Err(archon_workflow::WorkflowError::ControlCancelled(refused)) => {
            tracing::warn!(run_id, %refused, "stale executor stop refused at the locked write");
            Ok(())
        }
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

    /// Issue 316: a cancellation that reaches the run end while its launch
    /// executor still owns a run nobody cancelled is a refused pause. It is
    /// never written Cancelled: it is named and the run is paused. A real
    /// cancellation, a replaced executor and an unfenced run end are left
    /// to `stop`.
    #[test]
    fn a_refused_pause_of_an_owned_run_is_named_and_paused_never_cancelled() {
        let temp = tempfile::tempdir().unwrap();
        let store = WorkflowStore::project(temp.path());
        let spec = super::super::super::workflow_run_finalizer_tests::spec;
        let owned = store.create_run(spec()).unwrap();

        let paused = refused_while_owned(&store, &owned.id, Some(owned.generation), "refused");

        assert!(
            matches!(&paused, Some(archon_workflow::WorkflowError::ControlPaused(m)) if m.contains("must not happen")),
            "{paused:?}"
        );
        assert_eq!(
            store.load_state(&owned.id).unwrap().status,
            RunStatus::Paused
        );
        let events = std::fs::read_to_string(store.events_path(&owned.id)).unwrap();
        assert!(events.contains("run_end_refused_pause"), "{events}");

        let cancelled = store.create_run(spec()).unwrap();
        let lifecycle = LifecycleController::new(store.clone());
        lifecycle
            .apply(&cancelled.id, LifecycleAction::Cancel)
            .unwrap();
        let replaced = store.create_run(spec()).unwrap();
        lifecycle
            .apply(&replaced.id, LifecycleAction::Pause)
            .unwrap();
        lifecycle
            .apply(&replaced.id, LifecycleAction::Resume)
            .unwrap();
        for (run, expected) in [
            (&cancelled, Some(cancelled.generation)),
            (&replaced, Some(replaced.generation)),
            (&owned, None),
        ] {
            let left = refused_while_owned(&store, &run.id, expected, "refused");
            assert!(left.is_none(), "{}: {left:?}", run.id);
        }
    }
}
