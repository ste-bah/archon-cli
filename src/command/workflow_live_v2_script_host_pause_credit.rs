//! What a taken pause covers, and what it lets a resume do (Issue 261).
//!
//! A pause record names every attempt the run had recorded when the pause was
//! taken: call id, attempt and input hash. That set is the pause's credit.
//! Repeated host evaluations have separate host-owned occurrence slots: equal
//! candidate content never substitutes one evaluation's answer for another.
//!
//! - **Replay.** A resumed run replays the script from the top. A call whose
//!   slot still holds a covered attempt, asked with the input it was recorded
//!   with, answers from that record verbatim, whatever its status: a refused
//!   landing or a failed author call too. Re-asking them would put a model or
//!   judge's new answer into the history the script rebuilds, and the rebuilt
//!   loop could then stall somewhere else and spend the old pause there.
//! - **Credit.** The pause is passed only while every covered attempt still
//!   stands (not invalidated by a restart, not replaced by a later attempt)
//!   and the run has been resumed since (its generation moved on). Work
//!   redone after a restart therefore never spends a credit earned by the
//!   work it replaced: reaching the same point, it pauses again.
//!
//! Attempts in flight when the pause was taken (a placeholder still running,
//! a sibling's call the pause interrupted, a cancelled call) are not covered:
//! they run again on resume, as any interrupted call does.

use super::*;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct CoveredAttempt {
    pub(super) call_id: String,
    pub(super) attempt: u32,
    pub(super) input_hash: String,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(super) struct ScriptPauseRecord {
    pub(super) pause_id: String,
    pub(super) joined: bool,
    pub(super) event_seq: Option<u64>,
    /// The run's generation once this pause was in force.
    pub(super) generation: u64,
    pub(super) covered: Vec<CoveredAttempt>,
}

/// The attempts `records` (the store's slots) hold that a pause covers.
pub(super) fn covered_attempts(records: &[WorkflowV2CallRecord]) -> Vec<CoveredAttempt> {
    records
        .iter()
        .filter(|record| {
            record.invalidated_by.is_none()
                && !matches!(
                    record.status,
                    WorkflowV2Status::Running | WorkflowV2Status::Cancelled
                )
                && !interrupted(record)
        })
        .map(|record| CoveredAttempt {
            call_id: record.call.id.clone(),
            attempt: record.attempt,
            input_hash: record.input_hash.clone(),
        })
        .collect()
}

/// A record a pause or cancel stopped mid-flight carries the reason as text
/// (`workflow_live_v2_script_host_interrupt.rs`). A host command's own
/// `interrupted` field is a flag, `true` only when its process was stopped.
fn interrupted(record: &WorkflowV2CallRecord) -> bool {
    matches!(
        record.result.data.get("interrupted"),
        Some(serde_json::Value::String(_) | serde_json::Value::Bool(true))
    )
}

/// Whether the slot record for `covered.call_id` is still that attempt.
fn stands(covered: &CoveredAttempt, slots: &[WorkflowV2CallRecord]) -> bool {
    slots.iter().any(|record| {
        record.call.id == covered.call_id
            && record.attempt == covered.attempt
            && record.input_hash == covered.input_hash
            && record.invalidated_by.is_none()
    })
}

/// A pause's credit holds while every attempt it covers still stands.
pub(super) fn credit_holds(record: &ScriptPauseRecord, slots: &[WorkflowV2CallRecord]) -> bool {
    record.covered.iter().all(|covered| stands(covered, slots))
}

/// Every pause record of the run, unreadable ones skipped (evidence only: a
/// pause without a readable record is simply taken again).
pub(super) fn pause_records(
    store: &WorkflowStore,
    run_id: &str,
) -> archon_workflow::WorkflowResult<Vec<ScriptPauseRecord>> {
    let dir = store
        .run_dir(run_id)
        .join(super::workflow_live_v2_script_host_pause::SCRIPT_PAUSE_DIR);
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(WorkflowError::Io { path: dir, source }),
    };
    let mut records = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        match std::fs::read(&path)
            .ok()
            .and_then(|raw| serde_json::from_slice::<ScriptPauseRecord>(&raw).ok())
        {
            Some(record) => records.push(record),
            None => {
                tracing::warn!(path = %path.display(), "unreadable script pause record skipped")
            }
        }
    }
    Ok(records)
}

impl WorkflowScriptHost {
    /// Issue 358: an answer a limit cut short is an answer about that limit;
    /// after an upgrade changed it, the call runs again under the new one.
    pub(super) fn outcome_limits_hold(
        &self,
        record: &WorkflowV2CallRecord,
    ) -> archon_workflow::WorkflowResult<bool> {
        match self.runner.host_command_executor.as_ref() {
            Some(executor) => executor.outcome_limits_hold(record),
            None => Ok(true),
        }
    }

    /// The recorded answer to `execution` when a pause whose credit holds
    /// covers the attempt its slot holds, asked with the same input.
    pub(super) async fn replay_covered_attempt(
        &self,
        execution: &WorkflowV2CallExecution,
        input_hash: &str,
        generation: Option<u64>,
    ) -> archon_workflow::WorkflowResult<Option<String>> {
        let pauses = pause_records(&self.runner.workflow_store, &self.runner.run_id)?;
        if pauses.is_empty() {
            return Ok(None);
        }
        let Some(record) = self.runner.v2_store.load_call_record(&execution.call.id)? else {
            return Ok(None);
        };
        let wanted = CoveredAttempt {
            call_id: record.call.id.clone(),
            attempt: record.attempt,
            input_hash: record.input_hash.clone(),
        };
        if record.input_hash != input_hash || record.invalidated_by.is_some() {
            return Ok(None);
        }
        let slots = self.runner.v2_store.load_call_records()?;
        // Issue 360: a run seeded after an upgrade replays none of the history
        // before it, so a pause taken then credits nothing now (the history
        // floor itself: `predates_phase_seed`).
        let floor = self
            .phase_seed()
            .map(|seed| {
                seed["pause_generation_floor"]
                    .as_u64()
                    .ok_or_else(|| malformed_seed("pause_generation_floor"))
            })
            .transpose()?;
        let covered = pauses.iter().any(|pause| {
            floor.is_none_or(|floor| pause.generation > floor)
                && pause.covered.contains(&wanted)
                && credit_holds(pause, &slots)
        });
        if !covered || !self.outcome_limits_hold(&record)? {
            return Ok(None);
        }
        self.mark_reused(&record, generation).await?;
        Ok(Some(self.result_view(&record)?))
    }

    /// The phase seed this run started from, when it is a seeded resume.
    fn phase_seed(&self) -> Option<&serde_json::Value> {
        self.runner
            .script_args
            .as_ref()
            .map(|args| &args["phaseSeed"])
            .filter(|seed| !seed.is_null())
    }

    /// Issue 360: whether `record` is an attempt recorded before the seed this
    /// run started from. A seeded run replays none of that history: content-
    /// keyed calls (a gate asked a byte-identical candidate) would otherwise
    /// answer from an older runtime's record, an outage included.
    pub(super) fn predates_phase_seed(
        &self,
        record: &WorkflowV2CallRecord,
    ) -> archon_workflow::WorkflowResult<bool> {
        let Some(seed) = self.phase_seed() else {
            return Ok(false);
        };
        let floor = seed["history_attempts"]
            .as_object()
            .ok_or_else(|| malformed_seed("history_attempts"))?;
        Ok(match floor.get(&record.call.id) {
            None => false,
            Some(attempt) => {
                u64::from(record.attempt)
                    <= attempt
                        .as_u64()
                        .ok_or_else(|| malformed_seed("history_attempts"))?
            }
        })
    }
}

fn malformed_seed(field: &str) -> WorkflowError {
    WorkflowError::SpecInvalid(format!(
        "the phaseSeed argument has no readable {field}; the resume that seeds the run sets it"
    ))
}
