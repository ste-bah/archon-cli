//! Issue 262, round 8: recording a round and deciding the loop never fails
//! the run on its history.
//!
//! The progress ledger is rebuilt from the round records, healing what is
//! damaged (`progress::ProgressLedger::load_healing`): a record that will
//! not parse is quarantined and an event recorded, and the loop goes on
//! from the ledger's copy of its state. When that cannot be exact (no copy
//! survives), or the history or the new record cannot be read or written
//! (an I/O fault), the run PAUSES with the reason and the resume command:
//! never a failed call over a run left Running. Like an operator pause, a
//! history pause leaves no record of the round; the resume runs it again.
//! A loss of unknown state is reported by every execution until the pause
//! that reports it is recorded and acknowledges it (round 9).

use std::path::{Path, PathBuf};

use archon_workflow::v2::acceptance_stage::progress::{
    self, LoopDecision, QUARANTINE_DIR, QuarantinedRecordV1,
};
use archon_workflow::v2::acceptance_stage::{AcceptanceRoundRecordV1, write_round_record};
use archon_workflow::{WorkflowError, WorkflowStore};

/// The loop's decision after the round, and where its record was written.
pub(super) struct Decided {
    pub(super) decision: LoopDecision,
    pub(super) path: PathBuf,
}

/// Decides the loop after `record` from the healed history, then writes the
/// record and the ledger. `Err` is the control error the round ends with:
/// a pause of the run (or, for a round an operator pause and resume made
/// obsolete, the refusal to pause the newer generation).
pub(super) fn record_and_decide(
    store: &WorkflowStore,
    run_id: &str,
    generation: u64,
    run_dir: &Path,
    record: &mut AcceptanceRoundRecordV1,
) -> Result<Decided, WorkflowError> {
    let pause = |record: &AcceptanceRoundRecordV1, reason: String, quarantined| {
        pause_on_history(store, run_id, generation, record, &reason, quarantined)
    };
    let healed = match progress::ProgressLedger::load_healing(run_dir) {
        Ok(healed) => healed,
        Err(error) => {
            let reason = format!("the acceptance history could not be read ({error})");
            return Err(pause(record, reason, &[]));
        }
    };
    progress::record_quarantine_events(store, run_id, &healed.quarantined);
    let unknown = healed.unknown();
    if !unknown.is_empty() {
        let names: Vec<String> = (unknown.iter())
            .map(|lost| format!("{} (moved to {})", lost.original, lost.quarantined))
            .collect();
        let reason = format!(
            "acceptance record(s) {} would not parse and were quarantined; no copy of their failing state survives, so the rounds without progress cannot be counted exactly",
            names.join(", ")
        );
        let lost: Vec<QuarantinedRecordV1> = unknown.into_iter().cloned().collect();
        let paused = pause(record, reason, &lost);
        // Round 9: acknowledged only once the pause is recorded, so a death
        // before it leaves the next execution to pause on the loss again.
        if matches!(paused, WorkflowError::ControlPaused(_))
            && let Err(error) = progress::acknowledge_quarantined(run_dir, &lost)
        {
            tracing::warn!(%error, run_id, "the reported loss of quarantined acceptance records was not acknowledged; the next resume pauses on it again");
        }
        return Err(paused);
    }
    let mut ledger = healed.ledger;
    let decision = progress::decide_with(&mut ledger, record);
    record.final_round = decision.final_round;
    let path = match write_round_record(run_dir, record) {
        Ok(path) => path,
        Err(error) => {
            let reason = format!("the round record could not be written ({error})");
            return Err(pause(record, reason, &healed.quarantined));
        }
    };
    // The ledger is a copy (the records are authoritative); a copy that
    // did not save costs only the rebuild of a record damaged later.
    if let Err(error) = ledger.save(run_dir) {
        tracing::warn!(%error, run_id, "acceptance progress ledger copy not saved");
    }
    Ok(Decided { decision, path })
}

/// Pauses the run because its acceptance history could not be read or
/// rebuilt exactly, with the reason, what was quarantined and the resume.
fn pause_on_history(
    store: &WorkflowStore,
    run_id: &str,
    generation: u64,
    record: &AcceptanceRoundRecordV1,
    reason: &str,
    quarantined: &[QuarantinedRecordV1],
) -> WorkflowError {
    let resume = format!("archon workflow resume --live --yes {run_id}");
    let kept = if quarantined.is_empty() {
        String::new()
    } else {
        format!(
            " The damaged bytes and their evidence are kept in each round's {QUARANTINE_DIR}/ directory."
        )
    };
    let message = format!(
        "acceptance round {} (attempt {}): {reason}; the run is paused, not failed.{kept} Deal with the cause, then {resume}: the round runs again on the remaining evidence",
        record.round, record.attempt
    );
    let detail = serde_json::json!({
        "event": "acceptance_history_pause",
        "round": record.round,
        "attempt": record.attempt,
        "reason": reason,
        "quarantined": quarantined,
        "resume": resume,
    });
    match archon_workflow::control_pause::pause_with_evidence(store, run_id, generation, detail) {
        Ok(event) => {
            if let Err(error) = event {
                tracing::warn!(%error, "acceptance history pause event not recorded");
            }
            tracing::warn!(run_id, "{message}");
            WorkflowError::ControlPaused(message)
        }
        Err(error) => error,
    }
}
