use super::*;

#[test]
fn resume_terminal_stops_record_status_and_preserve_output() {
    use crate::command::workflow::workflow_decompose_reverify_tests::fixture;
    for status in [RunStatus::NeedsReview, RunStatus::Failed] {
        let fixture = fixture();
        let mut run = fixture.store.load_state(&fixture.run_id).unwrap();
        run.status = status;
        fixture.store.save_state(&run).unwrap();
        let output = "resume output with the terminal finding";
        let error = inspect_resume(
            &fixture.store,
            &fixture.run_id,
            run.generation,
            "report",
            1,
            output,
        )
        .unwrap_err();
        let text = format!("{error:#}");
        assert!(text.contains(&format!("{:?}", run.status)), "{text}");
        assert!(text.contains(output), "{text}");
        assert!(!text.contains("control or ownership changed"), "{text}");
        let (events, _) =
            archon_workflow::events::read_event_log(&fixture.store.events_path(&fixture.run_id))
                .unwrap();
        let event = events.last().unwrap();
        assert_eq!(event.kind, WorkflowEventKind::DecompositionReverifyRound);
        assert_eq!(
            event.detail["status"],
            serde_json::to_value(run.status).unwrap()
        );
        assert_eq!(event.detail["decision"], "ResumeStopped");
    }
}

#[test]
fn resume_pause_stops_record_the_actual_cause_and_output() {
    use crate::command::workflow::workflow_decompose_reverify_tests::fixture;
    for cause in [
        "no_progress",
        "host command unavailable",
        "author budget spent",
    ] {
        let fixture = fixture();
        let mut run = fixture.store.load_state(&fixture.run_id).unwrap();
        let generation = run.generation;
        WorkflowEventLog::new(fixture.store.clone())
            .emit(
                &run.id,
                fixture.store.next_event_seq(&run.id).unwrap(),
                WorkflowEventKind::Resumed,
                serde_json::json!({"generation": generation}),
            )
            .unwrap();
        run.status = RunStatus::Paused;
        run.generation += 1;
        fixture.store.save_state(&run).unwrap();
        WorkflowEventLog::new(fixture.store.clone())
            .emit(
                &run.id,
                fixture.store.next_event_seq(&run.id).unwrap(),
                WorkflowEventKind::Paused,
                serde_json::json!({"cause_kind": "script_pause", "cause_reason": cause}),
            )
            .unwrap();
        let error = inspect_resume(
            &fixture.store,
            &run.id,
            generation,
            "report",
            1,
            "the actual resume output",
        )
        .unwrap_err();
        let text = format!("{error:#}");
        assert!(text.contains(cause), "{text}");
        assert!(text.contains("the actual resume output"), "{text}");
        assert!(!text.contains("control or ownership changed"), "{text}");
        let (events, _) =
            archon_workflow::events::read_event_log(&fixture.store.events_path(&run.id)).unwrap();
        let stopped = events.last().unwrap();
        assert_eq!(stopped.detail["decision"], "ResumeStopped");
        assert_eq!(stopped.detail["cause"], cause);
    }
}

#[test]
fn resume_operator_pause_does_not_append_a_round_after_control() {
    use crate::command::workflow::workflow_decompose_reverify_tests::fixture;
    let fixture = fixture();
    let run = fixture.store.load_state(&fixture.run_id).unwrap();
    WorkflowEventLog::new(fixture.store.clone())
        .emit(
            &run.id,
            fixture.store.next_event_seq(&run.id).unwrap(),
            WorkflowEventKind::Resumed,
            serde_json::json!({"generation": run.generation}),
        )
        .unwrap();
    archon_workflow::LifecycleController::new(fixture.store.clone())
        .apply(&run.id, archon_workflow::LifecycleAction::Pause)
        .unwrap();
    let before = std::fs::read(fixture.store.events_path(&run.id)).unwrap();
    let error = inspect_resume(
        &fixture.store,
        &run.id,
        run.generation,
        "report",
        1,
        "resume observed operator pause",
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("control or ownership changed"));
    assert_eq!(
        std::fs::read(fixture.store.events_path(&run.id)).unwrap(),
        before
    );
}
