//! History replay for a resumed fixed run: see
//! `archon_workflow::v2::script::history_replay`.
use super::*;
use archon_workflow::v2::script::history_replay::replayable_history;

impl WorkflowScriptHost {
    /// The recorded result of a call that a later call over the same subject
    /// has superseded, when this is a fixed run resuming and the call arrives
    /// with the input it was recorded with. Its status is irrelevant: a
    /// refused or malformed attempt, or a freeze that carried findings, is
    /// history exactly as an accepted one is, and replaying it verbatim is
    /// what keeps the phase's budget and best artifact the same across a
    /// pause. A record a pause or cancel interrupted is no answer and never
    /// history. Agent calls and host commands alike; the live paths below never
    /// see a superseded record. A run seeded after an upgrade has no such
    /// history from before its seed (`predates_phase_seed`).
    pub(super) async fn replay_superseded_history(
        &self,
        execution: &WorkflowV2CallExecution,
        input_hash: &str,
        generation: Option<u64>,
    ) -> archon_workflow::WorkflowResult<Option<String>> {
        if self.runner.host_command_executor.is_none() {
            return Ok(None);
        }
        let Some(record) = self.runner.v2_store.load_call_record(&execution.call.id)? else {
            return Ok(None);
        };
        let records = self.runner.v2_store.load_call_records()?;
        // Batch H: history answers only the question it was recorded for.
        if self.answer_predates_question(execution, &record)? {
            return Ok(None);
        }
        // Issue 360: a seeded run replays no attempt recorded before its seed;
        // a landing the executor finds still live is the artifact on disk.
        let history = !self.predates_phase_seed(&record)?
            && replayable_history(&record, &records, input_hash);
        if !history && !self.landed_record_still_on_disk(&record, input_hash)? {
            return Ok(None);
        }
        if !self.outcome_holds(&record)? {
            return Ok(None);
        }
        if !self.refresh_audit_for_cache(&record).await? {
            return Ok(None);
        }
        self.mark_reused(&record, generation).await?;
        Ok(Some(match generation {
            Some(generation) => self.result_view_in_generation(&record, generation)?,
            None => self.result_view(&record)?,
        }))
    }

    /// Issue-250: a reused record read from the call's history (its last
    /// accepted record, displaced by a later attempt that a pause, a cancel
    /// or a dead host interrupted) goes back into the call's slot before it
    /// is credited, so every reader of the slot sees the answer that was
    /// reused. Always under the run lock, the lock every restart holds, and
    /// only while no restart has moved the restart epoch on since this
    /// session opened (any run kind, Issue-256) and, for a fixed run, while
    /// `generation` still owns the run. A slot record reused as it stands
    /// is left alone.
    pub(in super::super) fn restore_reused_record(
        &self,
        record: &WorkflowV2CallRecord,
        from_history: bool,
        generation: Option<u64>,
    ) -> archon_workflow::WorkflowResult<()> {
        if !from_history {
            return Ok(());
        }
        let run_id = &self.runner.run_id;
        self.runner.workflow_store.with_run_lock(run_id, |locked| {
            if let Some(expected) = generation {
                let current = locked.load_state(run_id)?.generation;
                if current != expected {
                    return Err(WorkflowError::ControlCancelled(format!(
                        "generation {expected} cannot restore the accepted record of call {} for run {run_id}; current generation is {current}",
                        record.call.id
                    )));
                }
            }
            // Issue 291: the epoch, and this session's executor.
            self.runner.v2_store.require_session_owner()?;
            self.runner.v2_store.restore_call_record(record)
        })?;
        tracing::info!(
            call_id = %record.call.id,
            attempt = record.attempt,
            "last accepted record restored from the call's history"
        );
        Ok(())
    }

    /// The last landing of a subject is not history, but it is not a question
    /// to ask again either while the executor finds it live -- identity,
    /// receipt, postcondition and terminal subject all still holding: its
    /// findings were the judge's answer about exactly this artifact, and only
    /// a non-deterministic judge would answer differently. Re-asking is how a
    /// resumed run lost an accepted contract. A receipt that no longer matches
    /// (an operator edit during the pause) falls through to live execution.
    fn landed_record_still_on_disk(
        &self,
        record: &WorkflowV2CallRecord,
        input_hash: &str,
    ) -> archon_workflow::WorkflowResult<bool> {
        if record.call.method != WorkflowV2HostMethod::HostCommand
            || record.invalidated_by.is_some()
            || record.input_hash != input_hash
            || record.result.data["publicationReceipt"].is_null()
        {
            return Ok(false);
        }
        let Some(executor) = self.runner.host_command_executor.as_ref() else {
            return Ok(false);
        };
        executor.record_is_live(record)
    }
}
