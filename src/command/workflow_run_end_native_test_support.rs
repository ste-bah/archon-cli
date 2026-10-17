use super::*;

pub(super) fn persist_native_terminal(fixture: &FrozenFixture, run_id: &str) {
    let mut run = fixture.store.load_state(run_id).unwrap();
    run.status = RunStatus::Completed;
    fixture.store.save_state(&run).unwrap();
    let mut record = archon_workflow::FinalizationRecordV1::new(
        WorkflowRunKind::AuthoredTaskWorkflow,
        WorkflowV2Status::Accepted,
        Some(fixture.snapshot.clone()),
    );
    record.mark_terminal_event_committed();
    fixture
        .store
        .write_run_json(run_id, FINALIZATION_RECORD_PATH, &record)
        .unwrap();
}
