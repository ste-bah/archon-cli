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
    self, LoopDecision, ProgressLedger, QUARANTINE_DIR, QuarantinedRecordV1,
};
use archon_workflow::v2::acceptance_stage::{AcceptanceRoundRecordV1, record_round};
use archon_workflow::{WorkflowError, WorkflowStore};

/// The loop's decision after the round, and where its record was written.
pub(super) struct Decided {
    pub(super) decision: LoopDecision,
    pub(super) path: PathBuf,
}

/// Why a round's record did not land.
enum Halt {
    /// The history or the record could not be read or written: the run
    /// pauses with the reason.
    Pause(String),
    /// Records quarantined with no copy of their state: the run pauses on
    /// the loss, and the pause acknowledges it.
    Lost(String, Vec<QuarantinedRecordV1>),
    /// Issue 316: this round's executor no longer owns the run (a resume
    /// replaced it). It stops and changes nothing; the newer owner records.
    Stop(WorkflowError),
}

impl From<WorkflowError> for Halt {
    fn from(error: WorkflowError) -> Self {
        Self::Pause(format!("the round record could not be written ({error})"))
    }
}

/// Decides the loop after `record` from the healed history, then writes the
/// record and the ledger. `Err` is the control error the round ends with:
/// a pause of the run (or, for a round an operator pause and resume made
/// obsolete, the refusal to pause or write for the newer generation).
///
/// Issue 316: the decision and the write happen under the recording-order
/// lock ([`record_round`]), fenced by executor ownership: an obsolete
/// executor's round lands nothing, and when another writer took this
/// round's attempt the record takes the next free one, decided on the
/// history that includes the other record. The owner never pauses for a
/// number.
///
/// Issue 320: a round whose stage context could not be resolved (no
/// execution recorded) in a run with no task set (`task_set` false) has
/// nothing the host can ever act on: no contract can be authored or run for
/// it, and no resume changes that. It is final, unevaluated (NeedsReview),
/// never a stall to pause on again and again. Every other round with an
/// operational error stays open, and repeats pause on the no-progress bound.
pub(super) fn record_and_decide(
    store: &WorkflowStore,
    run_id: &str,
    generation: u64,
    run_dir: &Path,
    record: &mut AcceptanceRoundRecordV1,
    task_set: bool,
) -> Result<Decided, WorkflowError> {
    let mut quarantined = Vec::new();
    let landed = record_round(run_dir, record, |record| {
        decide_locked(
            store,
            run_id,
            generation,
            run_dir,
            record,
            task_set,
            &mut quarantined,
        )
    });
    progress::record_quarantine_events(store, run_id, &quarantined);
    let pause = |record: &AcceptanceRoundRecordV1, reason: String, quarantined| {
        pause_on_history(store, run_id, generation, record, &reason, quarantined)
    };
    let (path, (decision, ledger)) = match landed {
        Ok(landed) => landed,
        Err(Halt::Stop(refused)) => return Err(refused),
        Err(Halt::Pause(reason)) => return Err(pause(record, reason, &quarantined)),
        Err(Halt::Lost(reason, lost)) => {
            let paused = pause(record, reason, &lost);
            // Round 9: acknowledged only once the pause is recorded, so a
            // death before it leaves the next execution to pause again.
            if matches!(paused, WorkflowError::ControlPaused(_))
                && let Err(error) = progress::acknowledge_quarantined(run_dir, &lost)
            {
                tracing::warn!(%error, run_id, "the reported loss of quarantined acceptance records was not acknowledged; the next resume pauses on it again");
            }
            return Err(paused);
        }
    };
    // The ledger is a copy (the records are authoritative); a copy that
    // did not save costs only the rebuild of a record damaged later.
    if let Err(error) = ledger.save(run_dir) {
        tracing::warn!(%error, run_id, "acceptance progress ledger copy not saved");
    }
    Ok(Decided { decision, path })
}

/// Under the order lock: fences the writer, heals the history and decides
/// the loop after `record`, which lands as its attempt when this returns
/// `Ok`. `quarantined` gets what the load moved, for its events.
fn decide_locked(
    store: &WorkflowStore,
    run_id: &str,
    generation: u64,
    run_dir: &Path,
    record: &mut AcceptanceRoundRecordV1,
    task_set: bool,
    quarantined: &mut Vec<QuarantinedRecordV1>,
) -> Result<(LoopDecision, ProgressLedger), Halt> {
    let run = (store.load_state(run_id)).map_err(|error| {
        Halt::Pause(format!(
            "the run state could not be read to check that this round still owns the run ({error})"
        ))
    })?;
    archon_workflow::control_pause::require_executor(&run, generation).map_err(Halt::Stop)?;
    let healed = ProgressLedger::load_healing(run_dir).map_err(|error| {
        Halt::Pause(format!(
            "the acceptance history could not be read ({error})"
        ))
    })?;
    quarantined.clone_from(&healed.quarantined);
    let unknown = healed.unknown();
    if !unknown.is_empty() {
        let names: Vec<String> = (unknown.iter())
            .map(|lost| {
                format!(
                    "{} (moved to {}: {})",
                    lost.original, lost.quarantined, lost.reason
                )
            })
            .collect();
        let reason = format!(
            "acceptance record(s) {} would not parse and were quarantined; no copy of their failing state survives, so the rounds without progress cannot be counted exactly",
            names.join(", ")
        );
        return Err(Halt::Lost(reason, unknown.into_iter().cloned().collect()));
    }
    let mut ledger = healed.ledger;
    let decision = if !task_set && record.execution.is_none() {
        ledger.observe(record);
        LoopDecision {
            final_round: true,
            escalate: false,
            stalled_rounds: 0,
            pause: None,
        }
    } else {
        progress::decide_with(&mut ledger, record)
    };
    record.final_round = decision.final_round;
    Ok((decision, ledger))
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
