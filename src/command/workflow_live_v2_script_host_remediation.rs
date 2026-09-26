//! Call-level replay for review remediation on a resume -- ordinal drift and
//! superseded rounds; see `archon_workflow::v2::script::resume_drift`. Split
//! out of `workflow_live_v2_script_host_exec.rs` to hold the 500-line ceiling.

use super::*;
use archon_workflow::v2::script::resume_drift::remediation_replay_record_escalating;
use archon_workflow::v2::script::resume_verdict::{
    is_remediation_fix, is_remediation_verdict, remediation_round_key,
    verdict_vouches_for_session_fix,
};

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
            ) && verdict_vouches_for_session_fix(record, &records, &self.runner.v2_store)
        };
        let in_session = |call_id: &str| self.runner.v2_store.in_session(call_id);
        let universe = self.runner.task_universe.as_ref();
        let escalates = |record: &WorkflowV2CallRecord| {
            archon_workflow::v2::script::remediation_escalation::buys_escalation(
                record,
                universe,
                self.repository_root(),
            )
        };
        Ok(remediation_replay_record_escalating(
            execution, &records, in_session, matches, escalates,
        )
        .cloned())
    }

    /// Whether a stored record may answer this session's call: a remediation
    /// verdict only while this session's fix was replayed from the fix that
    /// verdict judged (`resume_verdict`). Anything else may.
    pub(super) fn verdict_vouches(
        &self,
        record: &WorkflowV2CallRecord,
    ) -> archon_workflow::WorkflowResult<bool> {
        if !is_remediation_verdict(&record.call) {
            return Ok(true);
        }
        let records = self.runner.v2_store.load_call_records()?;
        Ok(verdict_vouches_for_session_fix(
            record,
            &records,
            &self.runner.v2_store,
        ))
    }

    /// Issue-107: an escalated round stands only on the host's own plan. A
    /// mismatch dispatches nothing and is answered, run or replayed alike,
    /// as a round that landed nothing. Issue-111: a re-verification stands
    /// only on the host's plan for this session's fix of its round; refused,
    /// it dispatches nothing and is answered as no verdict.
    pub(super) fn escalation_refused_view(
        &self,
        execution: &WorkflowV2CallExecution,
    ) -> archon_workflow::WorkflowResult<Option<String>> {
        use archon_workflow::v2::script::remediation_escalation::{
            escalation_refusal, refused_escalation_result, refused_reverify_result,
            reverify_refusal,
        };
        if let Some(reason) = escalation_refusal(
            execution,
            &self.runner.v2_store,
            self.runner.task_universe.as_ref(),
            self.repository_root(),
        ) {
            eprintln!("{reason}");
            return result_view_json_shaped(
                &refused_escalation_result(&reason),
                self.envelope_shape,
            )
            .map(Some);
        }
        let Some(reason) = reverify_refusal(
            execution,
            &self.runner.v2_store,
            self.runner.task_universe.as_ref(),
            self.repository_root(),
        ) else {
            return Ok(None);
        };
        eprintln!("{reason}");
        result_view_json_shaped(&refused_reverify_result(&reason), self.envelope_shape).map(Some)
    }

    /// A fix no record answered runs: its verdict must be asked again. A
    /// write fan-out whose branches are all replayed from one record
    /// overrides this (`branch_cache::remediation::note_fix_lineage`).
    pub(super) fn note_fix_runs(&self, execution: &WorkflowV2CallExecution) {
        if is_remediation_fix(&execution.call)
            && let Some(key) = remediation_round_key(&execution.call)
        {
            self.runner.v2_store.note_fix_lineage(&key, None);
        }
    }
}
