use super::*;

pub(super) fn finalize_generated_control(
    store: &WorkflowStore,
    run: &WorkflowRun,
    run_kind: archon_workflow::WorkflowRunKind,
    status: RunStatus,
    message: &str,
) -> archon_workflow::WorkflowResult<()> {
    super::super::workflow_live_v3_run_end::stop(
        store,
        &run.id,
        run_kind,
        status,
        message,
        Some(run.generation),
    )
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
    fn assert_control_evidence(action: LifecycleAction, status: RunStatus) {
        let temp = tempfile::tempdir().unwrap();
        let store = WorkflowStore::project(temp.path());
        let run = store
            .create_run(super::super::super::workflow_run_finalizer_tests::spec())
            .unwrap();
        LifecycleController::new(store.clone())
            .apply(&run.id, action)
            .unwrap();
        finalize_generated_control(
            &store,
            &run,
            archon_workflow::WorkflowRunKind::FixedOrSavedScript,
            status.clone(),
            "legitimate control stop",
        )
        .unwrap();
        let path = store.run_dir(&run.id).join("v2/finalization.json");
        assert!(
            path.exists(),
            "control finalization is missing for {status:?}"
        );
        let record: archon_workflow::FinalizationRecordV1 =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(record.terminal_status, status);
        assert!(record.terminal_event_committed);
        assert_eq!(store.load_state(&run.id).unwrap().status, status);
        let events = std::fs::read_to_string(store.events_path(&run.id)).unwrap();
        assert_eq!(events.matches("terminal_status").count(), 1);
        finalize_generated_control(
            &store,
            &run,
            archon_workflow::WorkflowRunKind::FixedOrSavedScript,
            status,
            "legitimate control stop",
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(store.events_path(&run.id)).unwrap(),
            events
        );
    }

    #[test]
    fn round4_legitimate_pause_writes_control_evidence() {
        assert_control_evidence(LifecycleAction::Pause, RunStatus::Paused);
    }

    #[test]
    fn round4_legitimate_cancel_writes_control_evidence() {
        assert_control_evidence(LifecycleAction::Cancel, RunStatus::Cancelled);
    }

    fn paused_run(actions: &[LifecycleAction]) -> (tempfile::TempDir, WorkflowStore, WorkflowRun) {
        let temp = tempfile::tempdir().unwrap();
        let store = WorkflowStore::project(temp.path());
        let run = store
            .create_run(super::super::super::workflow_run_finalizer_tests::spec())
            .unwrap();
        let lifecycle = LifecycleController::new(store.clone());
        for action in actions {
            lifecycle.apply(&run.id, action.clone()).unwrap();
        }
        (temp, store, run)
    }

    fn terminal_events(store: &WorkflowStore, run: &WorkflowRun) -> Vec<String> {
        std::fs::read_to_string(store.events_path(&run.id))
            .unwrap()
            .lines()
            .filter(|line| line.contains("terminal_status"))
            .map(str::to_string)
            .collect()
    }

    /// Issue-253 round 5: the executor saw the pause; the operator then
    /// cancelled. The executor's late control stop must not undo the cancel.
    #[test]
    fn round5_an_executor_pause_stop_cannot_replace_a_later_operator_cancel() {
        let (_temp, store, run) = paused_run(&[LifecycleAction::Pause, LifecycleAction::Cancel]);
        finalize_generated_control(
            &store,
            &run,
            archon_workflow::WorkflowRunKind::FixedOrSavedScript,
            RunStatus::Paused,
            "executor observed the pause",
        )
        .unwrap();
        assert_eq!(
            store.load_state(&run.id).unwrap().status,
            RunStatus::Cancelled
        );
        assert!(
            terminal_events(&store, &run)
                .iter()
                .all(|event| !event.contains("\"paused\"")),
            "{:?}",
            terminal_events(&store, &run)
        );
    }

    /// Issue-253 round 5: an ordinary executor failure while the operator has
    /// the run paused leaves the pause in place, resumable.
    #[test]
    fn round5_an_executor_failure_cannot_replace_an_operator_pause() {
        let (_temp, store, run) = paused_run(&[LifecycleAction::Pause]);
        super::super::super::workflow_live_v2_finalizer::finalize_run_status(
            &store,
            &run.id,
            archon_workflow::WorkflowRunKind::FixedOrSavedScript,
            RunStatus::Failed,
            "ordinary executor failure",
            Some(run.generation),
        )
        .unwrap();
        assert_eq!(store.load_state(&run.id).unwrap().status, RunStatus::Paused);
        assert!(
            terminal_events(&store, &run)
                .iter()
                .all(|event| !event.contains("\"failed\"")),
            "{:?}",
            terminal_events(&store, &run)
        );
    }
}
