//! Call-level replay for review remediation on a resume -- ordinal drift and
//! superseded rounds; see `archon_workflow::v2::script::resume_drift`. Split
//! out of `workflow_live_v2_script_host_exec.rs` to hold the 500-line ceiling.

use super::*;
use archon_workflow::v2::script::resume_drift::remediation_replay_record;

impl WorkflowScriptHost {
    /// A stored record this remediation call may replay although no record
    /// under its own id is reusable. Every candidate is held to the strict
    /// content check the ordinary path applies -- input hash, dynamic source
    /// fingerprint, scaffold -- computed for the call AS IT WOULD HAVE BEEN
    /// ISSUED under the candidate's ordinal.
    pub(super) fn remediation_replay(
        &self,
        execution: &WorkflowV2CallExecution,
    ) -> archon_workflow::WorkflowResult<Option<WorkflowV2CallRecord>> {
        // A dynamic write fan-out is never replayed whole (the audit gate
        // admits cached write credit per item); its branches decide.
        let dynamic_write =
            execution.call.write_mode.is_some() && execution.call.options.target_files_from_item;
        if dynamic_write
            || !archon_workflow::v2::script::resume_drift::is_remediation_call(&execution.call)
        {
            return Ok(None);
        }
        let records = self.runner.v2_store.load_call_records()?;
        let matches = |candidate: &WorkflowV2CallExecution, record: &WorkflowV2CallRecord| {
            let metadata = dynamic_wave_source_metadata(
                candidate,
                self.runner.task_universe.as_ref(),
                self.runner.runtime.target_repository_root.as_deref(),
            );
            if metadata.source_metadata_required && metadata.source_fingerprint.is_none() {
                return false;
            }
            let input_hash = input_hash_with_source_fingerprint(
                &candidate.input,
                metadata.source_fingerprint.as_deref(),
            );
            record.matches_input_for_source_and_scaffold(
                &input_hash,
                metadata.source_fingerprint.as_deref(),
                Some(&self.scaffold_hash),
            )
        };
        let in_session = |call_id: &str| self.runner.v2_store.in_session(call_id);
        Ok(remediation_replay_record(execution, &records, in_session, matches).cloned())
    }
}
