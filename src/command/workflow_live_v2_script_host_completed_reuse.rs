//! Completed-task record reuse under ordinal drift on a resume. Split out of
//! `workflow_live_v2_script_host_exec.rs` to hold the 500-line ceiling.

use super::*;

impl WorkflowScriptHost {
    /// Find an accepted stored record to reuse for a call whose task is already
    /// completed but whose ordinal-suffixed id shifted on re-run. Matches by
    /// task + kind (verify vs implement/remediate), preferring the latest
    /// accepted attempt; a remediation call also by the question it asks. Only scans when the call actually belongs to a
    /// completed task, so non-completed calls pay no cost.
    pub(super) fn reusable_completed_task_record(
        &self,
        execution: &WorkflowV2CallExecution,
    ) -> archon_workflow::WorkflowResult<Option<WorkflowV2CallRecord>> {
        let completed = &self.runner.resume_completed_ids;
        if completed.is_empty() {
            return Ok(None);
        }
        // Only v3 implement/verify calls reuse this way, and only against
        // records of the SAME v3 family — never a stale decomposed record.
        let Some(want_family) = v3_call_family(&execution.call.id) else {
            return Ok(None);
        };
        // Which completed task does this call belong to? Match by the canonical
        // task token embedded in the call id.
        let call_id_lower = execution.call.id.to_ascii_lowercase();
        let Some(task_token) = completed
            .iter()
            .map(|task| task.to_ascii_lowercase())
            .find(|token| call_id_lower.contains(token.as_str()))
        else {
            return Ok(None);
        };
        // No evidence check here: implement/remediate records carry no task-id
        // evidence (only verify records do); the task is already in `completed`,
        // the family is fixed, and accepted+valid suffices.
        let remediation =
            archon_workflow::v2::script::resume_drift::is_remediation_call(&execution.call);
        let mut best: Option<WorkflowV2CallRecord> = None;
        for record in self.runner.v2_store.load_call_records()? {
            if v3_call_family(&record.call.id) != Some(want_family) {
                continue;
            }
            if !record.call.id.to_ascii_lowercase().contains(&task_token) {
                continue;
            }
            if !(is_reusable_status(record.status)
                && record.invalidated_by.is_none()
                && record.result.validate().is_ok())
            {
                continue;
            }
            // This path CANNOT key on the input hash: it exists precisely
            // because the call arrives under a new ordinal-suffixed id, and the
            // call id is part of the hashed input, so the hashes can never match
            // by construction. Bound it instead — a task whose upstream work has
            // been redone in this run must not be served from a record produced
            // before that redo.
            if self.hash_free_reuse_stale(&record) {
                continue;
            }
            // Issue 265: a remediation call is held to the checks every other
            // reuse path applies. Only a record of the same question answers
            // it: never an implement record, a record of other findings, or
            // one older than the question's latest observation.
            if remediation
                && !(asks_the_same(&record.call, &execution.call)
                    && !self.answer_predates_question(execution, &record)?
                    && self.verdict_vouches(&record)?)
            {
                continue;
            }
            if best
                .as_ref()
                .is_none_or(|current| record.attempt >= current.attempt)
            {
                best = Some(record);
            }
        }
        Ok(best)
    }
}
