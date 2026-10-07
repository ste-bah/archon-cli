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
    /// Issue 337: taken by the host ([`HostPauseCoverage`]), never passed.
    /// Each covered attempt replays while IT stands; one changed slot never
    /// voids the replay of the others.
    #[serde(default)]
    pub(super) host_taken: bool,
    /// Issue 337: for each covered UNPUBLISHED host-command outcome, the
    /// digest of what its gate reads, as it was when this host pause was
    /// taken (`workflow_live_v2_script_host_pause_judged`).
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub(super) judged_at_pause: std::collections::BTreeMap<String, String>,
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

/// A pause's credit holds while every attempt it covers still stands. A
/// host-taken record is never passed: each covered attempt replays while IT
/// stands (the replay checks that), so one changed slot voids no other.
pub(super) fn credit_holds(record: &ScriptPauseRecord, slots: &[WorkflowV2CallRecord]) -> bool {
    record.host_taken || record.covered.iter().all(|covered| stands(covered, slots))
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

/// Issue 337: what a pause or a terminal stop the HOST takes for a fixed
/// script (a crash, a fault at the run boundary, a deliberate stop) covers:
/// the snapshot a `w.pause` records, less every record that carries no
/// verdict (a dispatch error or a never-ran fault, which a resume asks again,
/// so a host fault never replays into the same pause). Written as one more
/// pause record, so a resume replays every covered verdict -- an unpublished
/// refusal too, which no other reuse path answers -- through
/// [`WorkflowScriptHost::replay_covered_attempt`], each while its own slot
/// still holds it under the same input. Taken before the transition, recorded
/// once it is in force, under a name no script pause id maps to
/// (`pause_record_path` always ends in a 16-hex digest); never "passed".
pub(in super::super::super) struct HostPauseCoverage {
    covered: Vec<CoveredAttempt>,
    /// The covered unpublished host-command outcomes, digested at `record`.
    judged: Vec<(String, archon_workflow::HostCommandRequest)>,
    executor: Option<Arc<HostCommandExecutor>>,
}

type HostCommandExecutor =
    dyn crate::command::workflow_host_command_exec::WorkflowHostCommandExecutor;

impl HostPauseCoverage {
    pub(in super::super::super) fn snapshot(
        v2_store: &WorkflowV2ResultStore,
        executor: Option<&Arc<HostCommandExecutor>>,
    ) -> Self {
        let records = v2_store
            .load_call_records()
            .map(|mut records| {
                // A dispatch error or a never-ran fault is no answer:
                // a resume asks it again (Issue 337 round 3).
                records.retain(|record| {
                    !archon_workflow::v2::host_fault::result_carries_no_verdict(
                        &record.call.id,
                        &record.result,
                    )
                });
                records
            })
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "host pause covers no recorded attempt");
                Vec::new()
            });
        let covered = covered_attempts(&records);
        let judged = super::workflow_live_v2_script_host_pause_judged::unpublished_requests(
            &records, &covered,
        );
        Self {
            covered,
            judged,
            executor: executor.cloned(),
        }
    }

    /// Records the coverage of the pause `cause` took. Losing it costs the
    /// replay only: a resume then asks those calls again, as before.
    pub(in super::super::super) fn record(
        self,
        store: &WorkflowStore,
        run_id: &str,
        cause: &str,
        event_seq: Option<u64>,
    ) {
        let generation = match store.load_state(run_id) {
            Ok(run) => run.generation,
            Err(error) => {
                tracing::warn!(%error, run_id, "host pause coverage not recorded");
                return;
            }
        };
        let pause_id = format!("host-{cause}-g{generation}");
        let path = format!(
            "{}/{pause_id}.json",
            super::workflow_live_v2_script_host_pause::SCRIPT_PAUSE_DIR
        );
        // In the pause's lock section: the content as the pause leaves it.
        let judged_at_pause = super::workflow_live_v2_script_host_pause_judged::digests_now(
            self.executor.as_deref(),
            &self.judged,
        );
        let record = ScriptPauseRecord {
            pause_id,
            joined: false,
            event_seq,
            generation,
            covered: self.covered,
            host_taken: true,
            judged_at_pause,
        };
        if let Err(error) = store.write_run_json(run_id, &path, &record) {
            tracing::warn!(%error, run_id, "host pause coverage not recorded");
        }
    }
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
        // Issue 337: a covered answer replays only while it is still one.
        if !self
            .covered_answer_holds(&record, &pauses, &wanted, &slots)
            .await?
        {
            return Ok(None);
        }
        self.mark_reused(&record, generation).await?;
        // Issue 337: a replayed terminal verdict stops the script again.
        if terminal_stop_for_call(&record.call, record.status) {
            return Err(self.stop_on_terminal_call(&record).await);
        }
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

impl WorkflowScriptHost {
    /// Issue 337: whether a covered record may still answer its call. A
    /// `w.pause` whose credit holds keeps its contract: its covered attempts
    /// replay verbatim, failed calls too. A HOST-taken record answers only by
    /// the checks every other reuse path applies. A record without a verdict
    /// (a dispatch error or a never-ran fault, marked or in an older binary's
    /// shape) never replays. A host command's PUBLISHED outcome replays only
    /// while it is still what is on disk (`record_is_live`); an unpublished
    /// one only while its identity is unchanged and what its gate reads is
    /// what it was at the last pause covering it
    /// (`workflow_live_v2_script_host_pause_judged`), so a crash a disk state
    /// caused heals once the disk is repaired. Any other call passes the audit
    /// admission of a cached answer.
    async fn covered_answer_holds(
        &self,
        record: &WorkflowV2CallRecord,
        pauses: &[ScriptPauseRecord],
        wanted: &CoveredAttempt,
        slots: &[WorkflowV2CallRecord],
    ) -> archon_workflow::WorkflowResult<bool> {
        if pauses.iter().any(|pause| {
            !pause.host_taken && pause.covered.contains(wanted) && credit_holds(pause, slots)
        }) {
            return Ok(true);
        }
        if archon_workflow::v2::host_fault::result_carries_no_verdict(
            &record.call.id,
            &record.result,
        ) {
            return Ok(false);
        }
        if record.call.method != WorkflowV2HostMethod::HostCommand {
            return self.refresh_audit_for_cache(record).await;
        }
        let Some(executor) = self.runner.host_command_executor.as_ref() else {
            return Ok(false);
        };
        if record.result.data["publicationReceipt"].is_null() {
            Ok(self.judged_at_resume().unchanged_since_pause(wanted)
                && crate::command::workflow_host_command_judged_inputs::identity_holds(
                    executor.as_ref(),
                    record,
                )?)
        } else {
            executor.record_is_live(record)
        }
    }
}
