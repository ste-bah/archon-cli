//! The acceptance call's own record, read at the run end so that damage
//! never fails the run and is never read as "the stage never ran"
//! (Issue 313).
//!
//! The call record and the round record it names are two copies of one
//! round. Group E (Issue 262, round 9) rebuilds a lost round record from the
//! call's result; this is the other direction. A call slot that does not
//! read back as the call's record -- not JSON, the wrong shape, another
//! call's record -- is quarantined with evidence by the store
//! ([`WorkflowV2ResultStore::load_call_slot_healing`]), and the record is
//! rebuilt from the round record: the newest attempt of the call's round,
//! when it is whole and was written by this call. When it is not, the run
//! PAUSES with the reason. A resume runs the acceptance stage again (an
//! acceptance call is never replayed from the store), so the new round and
//! its new call record heal the run.

use archon_workflow::v2::acceptance_stage::progress::LoopDecision;
use archon_workflow::v2::acceptance_stage::{
    AcceptanceRoundRecordV1, attempt_file_name, next_attempt, relative_record_path, round_dir,
};
use archon_workflow::v2::{QuarantinedCallRecordV1, WorkflowV2CallSlot};
use archon_workflow::{
    WorkflowError, WorkflowEventKind, WorkflowEventLog, WorkflowResult, WorkflowStore,
    WorkflowV2CallRecord, WorkflowV2HostCall, WorkflowV2ResultStore,
};

/// The record of the acceptance `call`: its own, or one rebuilt from its
/// round record when its own is damaged. `None` when it recorded nothing.
/// `Err(ControlPaused)` when neither copy is whole, or the slot cannot be
/// read or quarantined (an I/O fault): the run is paused.
pub(super) fn acceptance_call_record(
    store: &WorkflowStore,
    run_id: &str,
    v2_store: &WorkflowV2ResultStore,
    call: &WorkflowV2HostCall,
) -> WorkflowResult<Option<WorkflowV2CallRecord>> {
    let (evidence, fresh) = match v2_store.load_call_slot_healing(&call.id) {
        Ok(WorkflowV2CallSlot::Whole(record)) => return Ok(Some(*record)),
        Ok(WorkflowV2CallSlot::Empty) => return Ok(None),
        Ok(WorkflowV2CallSlot::Damaged { evidence, fresh }) => (evidence, fresh),
        Err(error @ WorkflowError::Io { .. }) => {
            let slot = v2_store.result_path(&call.id);
            let named = relative_record_path(&store.run_dir(run_id), &slot);
            let reason = format!("the acceptance call record {named} cannot be read ({error})");
            return Err(pause(store, run_id, &named, &reason));
        }
        Err(error) => return Err(error),
    };
    let run_dir = store.run_dir(run_id);
    let rebuilt = rebuild(&run_dir, v2_store, call);
    if fresh {
        quarantined(store, run_id, &evidence, rebuilt.as_ref().err());
    }
    match rebuilt {
        Ok(record) => {
            tracing::warn!(
                run_id,
                call_id = %call.id,
                quarantined = %evidence.quarantined,
                "the acceptance call record is damaged; it is rebuilt from its round record"
            );
            Ok(Some(record))
        }
        Err(missing) => {
            let reason = format!(
                "the acceptance call record {} is damaged ({}; quarantined at {}), and {missing}",
                evidence.original, evidence.reason, evidence.quarantined
            );
            Err(pause(store, run_id, &evidence.original, &reason))
        }
    }
}

/// `call`'s record rebuilt from the newest attempt of its round, when that
/// record is whole and is this call's; `Err` says why it is not.
fn rebuild(
    run_dir: &std::path::Path,
    v2_store: &WorkflowV2ResultStore,
    call: &WorkflowV2HostCall,
) -> Result<WorkflowV2CallRecord, String> {
    let round = (call.options.extra.get("round"))
        .and_then(serde_json::Value::as_u64)
        .and_then(|round| u32::try_from(round).ok())
        .filter(|round| *round >= 1)
        .ok_or("the call names no round, so no round record can stand in for it")?;
    // One past the newest attempt on disk, a quarantined one included.
    let attempt = next_attempt(run_dir, round) - 1;
    if attempt == 0 {
        return Err(format!("round {round} has no record to rebuild it from"));
    }
    let path = round_dir(run_dir, round).join(attempt_file_name(attempt));
    let named = relative_record_path(run_dir, &path);
    let bytes = std::fs::read(&path)
        .map_err(|error| format!("its round's newest record {named} cannot be read ({error})"))?;
    let record: AcceptanceRoundRecordV1 = serde_json::from_slice(&bytes)
        .map_err(|error| format!("its round's newest record {named} will not parse ({error})"))?;
    if record.call_id != call.id || (record.round, record.attempt) != (round, attempt) {
        return Err(format!(
            "its round's newest record {named} is call {}'s round {} attempt {}",
            record.call_id, record.round, record.attempt
        ));
    }
    // The answer the stage gave for this round, rebuilt by the stage's own
    // rule; the loop's escalation counts are the script's, not the gate's.
    let decision = LoopDecision {
        final_round: record.final_round,
        escalate: false,
        stalled_rounds: 0,
        pause: None,
    };
    let mut result =
        super::super::workflow_live_v3_acceptance::result_for(&record, &named, &decision);
    result.data["rebuilt_from_round_record"] = named.into();
    Ok(WorkflowV2CallRecord::new(
        v2_store.run_id(),
        call.clone(),
        0,
        String::new(),
        result,
        Vec::new(),
    ))
}

/// Records the quarantine in the run's events (best effort: the evidence
/// file beside the bytes is the durable record).
fn quarantined(
    store: &WorkflowStore,
    run_id: &str,
    evidence: &QuarantinedCallRecordV1,
    not_rebuilt: Option<&String>,
) {
    let mut detail = serde_json::to_value(evidence).unwrap_or_default();
    detail["event"] = "acceptance_call_record_quarantined".into();
    detail["rebuilt_from_round_record"] = not_rebuilt.is_none().into();
    detail["not_rebuilt"] = not_rebuilt.cloned().into();
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
