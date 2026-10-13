use super::*;

pub(super) use super::super::workflow_live_v2_fixed_run::lost_ownership::{
    lost_ownership, lost_ownership_report,
};

/// The ending of a generated run its script stopped with run control
/// `error`. An executor a resume replaced stops nothing and reports that it
/// lost ownership (Issue 329); the owner records the pause or cancel.
pub(super) fn generated_control_report(
    store: &WorkflowStore,
    run: &WorkflowRun,
    run_kind: archon_workflow::WorkflowRunKind,
    error: WorkflowError,
) -> archon_workflow::WorkflowResult<String> {
    let (status, message) = match error {
        WorkflowError::ControlPaused(message) => (RunStatus::Paused, message),
        WorkflowError::ControlCancelled(message) => (RunStatus::Cancelled, message),
        other => return Err(other),
    };
    if lost_ownership(store, &run.id, run.generation).is_some() {
        return Ok(lost_ownership_report("Workflow", &run.id, &message));
    }
    if let Err(error) = finalize_generated_control(store, run, run_kind, status.clone(), &message) {
        return Err(unrecorded_control_stop(
            store, run, &status, &message, error,
        ));
    }
    Ok(if status == RunStatus::Paused {
        format!(
            "Workflow paused: {}\n{}\nResume with: /workflow resume --live {}\n",
            run.id, message, run.id
        )
    } else {
        format!("Workflow cancelled: {}\n{}\n", run.id, message)
    })
}

/// #289: the control stop was decided but its run-end record failed -- the
/// terminal event, when the event log cannot be written, or the finalization
/// record. The error
/// stays an error (the run's evidence is incomplete), but it no longer hides
/// the stop: it says what state the run is in, read back from `state.json`,
/// names the file that refused the write (the error's own path), and keeps the stop's own message and
/// the resume command.
fn unrecorded_control_stop(
    store: &WorkflowStore,
    run: &WorkflowRun,
    status: &RunStatus,
    message: &str,
    error: WorkflowError,
) -> WorkflowError {
    let state_path = store.state_path(&run.id);
    let saved = match store.load_state(&run.id) {
        Ok(saved) if saved.status == *status => format!(
            "workflow run {} is {} (saved in {})",
            run.id,
            status_word(status),
            state_path.display()
        ),
        Ok(saved) => format!(
            "workflow run {} was to be {}, but {} holds status {:?}",
            run.id,
            status_word(status),
            state_path.display(),
            saved.status
        ),
        Err(read) => format!(
            "workflow run {} was to be {}, but its state could not be read back from {}: {read}",
            run.id,
            status_word(status),
            state_path.display()
        ),
    };
    tracing::error!(run_id = %run.id, %error, "control stop not fully recorded");
    // `error` names the file that refused the write (`io error at <path>`).
    WorkflowError::port(format!(
        "{saved}, but its run-end record was not fully written: {error}\n{message}\nResume with: archon workflow resume --live --yes {}",
        run.id
    ))
}

fn status_word(status: &RunStatus) -> &'static str {
    match status {
        RunStatus::Paused => "paused",
        RunStatus::Cancelled => "cancelled",
        _ => "stopped",
    }
}

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

    /// #289 item 5: the terminal event of a saved pause cannot be written.
    /// The pause stays saved, and the report names it, the file that refused
    /// the event, the pause's own message and the resume command -- never a
    /// bare io error that hides the pause.
    #[test]
    fn a_pause_whose_terminal_event_cannot_be_written_is_reported_with_the_file() {
        let (_temp, store, run) = paused_run(&[LifecycleAction::Pause]);
        let events = store.events_path(&run.id);
        std::fs::remove_file(&events).unwrap();
        std::fs::create_dir(&events).unwrap();
        let outcome = generated_control_report(
            &store,
            &run,
            archon_workflow::WorkflowRunKind::FixedOrSavedScript,
            WorkflowError::ControlPaused(
                "pause 'p-1' was saved, but its evidence event was not recorded".into(),
            ),
        );
        let message = match outcome {
            Err(error) => error.to_string(),
            Ok(report) => panic!("a stop that was not fully recorded is an error: {report}"),
        };
        for needle in [
            "is paused".to_string(),
            store.state_path(&run.id).display().to_string(),
            events.display().to_string(),
            "run-end record was not fully written".to_string(),
            "pause 'p-1' was saved, but its evidence event was not recorded".to_string(),
            format!("archon workflow resume --live --yes {}", run.id),
        ] {
            assert!(
                message.contains(&needle),
                "missing {needle:?} in: {message}"
            );
        }
        assert_eq!(store.load_state(&run.id).unwrap().status, RunStatus::Paused);
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
