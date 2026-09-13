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
    /// pause. Agent calls and host commands alike; the live paths below never
    /// see a superseded record.
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
        if !replayable_history(&record, &records, input_hash)
            && !self.landed_record_still_on_disk(&record, input_hash)?
        {
            return Ok(None);
        }
        if !self.refresh_audit_for_cache(&record).await? { return Ok(None); }
        self.mark_reused(&record, generation).await?;
        Ok(Some(self.result_view(&record.result)?))
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
