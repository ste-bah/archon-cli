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
        if !replayable_history(
            &record,
            &self.runner.v2_store.load_call_records()?,
            input_hash,
        ) {
            return Ok(None);
        }
        self.mark_reused(&record, generation).await?;
        Ok(Some(result_view_json(&record.result)?))
    }
}
