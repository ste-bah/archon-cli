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
}
