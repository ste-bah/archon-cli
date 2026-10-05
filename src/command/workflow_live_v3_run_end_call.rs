//! The acceptance call's own record, read at the run end so that damage
//! never fails the run and is never read as "the stage never ran"
//! (Issue 313).
//!
//! A call slot that does not read back as the call's record -- not JSON,
//! the wrong shape, another call's record -- is quarantined with evidence by
//! the store ([`WorkflowV2ResultStore::load_call_slot_healing`]) and the run
//! PAUSES with the reason; so does a slot the file system will not hand
//! over. Nothing stands in for the lost record (round 2): the call's id is
//! the same on every execution and a round record carries no session, so
//! the newest round of the call can be a stale pass from before a later
//! execution that failed. A resume runs the acceptance stage again (an
//! acceptance call is never replayed from the store,
//! `workflow_live_v2_script_host_exec.rs`), and its new round and new call
//! record heal the run.

use archon_workflow::v2::acceptance_stage::relative_record_path;
use archon_workflow::v2::{QuarantinedCallRecordV1, WorkflowV2CallSlot};
use archon_workflow::{
    WorkflowError, WorkflowEventKind, WorkflowEventLog, WorkflowResult, WorkflowStore,
    WorkflowV2CallRecord, WorkflowV2HostCall, WorkflowV2ResultStore,
};

/// The record of the acceptance `call`; `None` when it recorded nothing.
/// `Err(ControlPaused)` when the record is damaged (now quarantined) or the
/// slot cannot be read or quarantined (an I/O fault): the run is paused.
pub(super) fn acceptance_call_record(
    store: &WorkflowStore,
    run_id: &str,
    v2_store: &WorkflowV2ResultStore,
    call: &WorkflowV2HostCall,
) -> WorkflowResult<Option<WorkflowV2CallRecord>> {
    let run_dir = store.run_dir(run_id);
    let evidence = match v2_store.load_call_slot_healing(&call.id) {
        Ok(WorkflowV2CallSlot::Whole(record)) => return Ok(Some(*record)),
        Ok(WorkflowV2CallSlot::Empty) => return Ok(None),
        Ok(WorkflowV2CallSlot::Damaged(evidence)) => evidence,
        Err(error @ WorkflowError::Io { .. }) => {
            let named = relative_record_path(&run_dir, &v2_store.result_path(&call.id));
            let reason = format!("the acceptance call record {named} cannot be read ({error})");
            return Err(pause(store, run_id, &named, &reason));
        }
        Err(error) => return Err(error),
    };
    quarantined(store, run_id, &evidence);
    let named = relative_record_path(&run_dir, &v2_store.result_path(&call.id));
    let reason = format!(
        "the acceptance call record {named} is damaged ({}); it is quarantined at {}",
        evidence.reason, evidence.quarantined
    );
    Err(pause(store, run_id, &named, &reason))
}

/// Records the quarantine in the run's events (best effort: the evidence
/// file beside the bytes is the durable record).
fn quarantined(store: &WorkflowStore, run_id: &str, evidence: &QuarantinedCallRecordV1) {
    let mut detail = serde_json::to_value(evidence).unwrap_or_default();
    detail["event"] = "acceptance_call_record_quarantined".into();
    let kind = WorkflowEventKind::AcceptanceRecordQuarantined;
    if let Err(error) = store
        .next_event_seq(run_id)
        .and_then(|seq| WorkflowEventLog::new(store.clone()).emit(run_id, seq, kind, detail))
    {
        tracing::warn!(%error, run_id, "acceptance call quarantine event not recorded");
    }
}

/// Pauses `run_id`, owned by its current generation, because the final
/// gate cannot be judged (`reason`, about the record at `named`); the error
/// the finalization ends with. The resume runs the acceptance stage again.
pub(super) fn pause(
    store: &WorkflowStore,
    run_id: &str,
    named: &str,
    reason: &str,
) -> WorkflowError {
    let generation = match store.load_state(run_id) {
        Ok(run) => run.generation,
        Err(error) => return error,
    };
    let resume = format!("archon workflow resume --live --yes {run_id}");
    let detail = serde_json::json!({
        "event": "acceptance_gate_pause", "record_path": named, "reason": reason, "resume": resume,
    });
    match archon_workflow::control_pause::pause_with_evidence(store, run_id, generation, detail) {
        Ok(event) => {
            if let Err(error) = event {
                tracing::warn!(%error, run_id, "acceptance gate pause event not recorded");
            }
            let message = format!(
                "the final acceptance gate cannot be judged: {reason}; run {run_id} is paused, not failed. {resume} runs the acceptance stage again and the gate is judged on its new round"
            );
            tracing::warn!(run_id, "{message}");
            WorkflowError::ControlPaused(message)
        }
        Err(error) => error,
    }
}
