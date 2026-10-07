use super::QuarantinedRecordV1;

pub fn record_quarantine_events(
    store: &crate::WorkflowStore,
    run_id: &str,
    quarantined: &[QuarantinedRecordV1],
) {
    record_quarantine_events_owned(
        store,
        run_id,
        crate::control_pause::PauseOwner::Unfenced,
        quarantined,
    );
}

/// Records one `acceptance_record_quarantined` event per record in the
/// run's events, under the run lock. Best effort: the evidence file beside
/// the bytes is the durable record.
pub fn record_quarantine_events_owned(
    store: &crate::WorkflowStore,
    run_id: &str,
    owner: crate::control_pause::PauseOwner,
    quarantined: &[QuarantinedRecordV1],
) {
    for record in quarantined {
        let detail = serde_json::to_value(record).unwrap_or_default();
        let emitted = store.with_run_lock(run_id, |locked| {
            owner.require_writer(&locked.load_state(run_id)?)?;
            let seq = locked.next_event_seq(run_id)?;
            crate::WorkflowEventLog::new(locked.clone()).emit(
                run_id,
                seq,
                crate::WorkflowEventKind::AcceptanceRecordQuarantined,
                detail.clone(),
            )
        });
        if let Err(error) = emitted {
            tracing::warn!(%error, record = %record.original, "quarantine event not recorded");
        }
    }
}
