//! The persisted HostCommand executor contract the script bridge calls.
use archon_workflow::{
    HostCommandRequest, HostCommandResult, WorkflowResult, WorkflowV2CallRecord,
};
use async_trait::async_trait;

#[async_trait]
pub(crate) trait WorkflowHostCommandExecutor: Send + Sync {
    fn call_identity(&self, request: &HostCommandRequest) -> WorkflowResult<String>;
    /// Issue 337: the digest of the content a call of `request` judges
    /// (`workflow_host_command_judged_inputs`), recorded at a host pause and
    /// read again at the resume; `None`: an unpublished outcome never replays.
    fn judged_inputs(&self, _request: &HostCommandRequest) -> WorkflowResult<Option<String>> {
        Ok(None)
    }

    fn record_is_reusable(&self, record: &WorkflowV2CallRecord) -> WorkflowResult<bool>;
    /// Whether the record's landed outcome is still exactly what is on disk,
    /// findings or not: identity, receipt, postcondition and terminal subject.
    /// Defaults to the stricter reuse test.
    fn record_is_live(&self, record: &WorkflowV2CallRecord) -> WorkflowResult<bool> {
        self.record_is_reusable(record)
    }
    /// Whether a recorded outcome that a limit cut short was cut by the
    /// limits this build applies; replay paths ask before answering from it.
    fn outcome_limits_hold(&self, _record: &WorkflowV2CallRecord) -> WorkflowResult<bool> {
        Ok(true)
    }
    /// The limits a call of `request` runs under, stamped into its outcome.
    fn limits_fingerprint(
        &self,
        _request: &HostCommandRequest,
    ) -> WorkflowResult<Option<serde_json::Value>> {
        Ok(None)
    }
    /// Issue 361: the logic version a call of `request` is judged by,
    /// stamped into its outcome; `None` for an executor that versions none.
    fn logic_version(&self, _request: &HostCommandRequest) -> WorkflowResult<Option<u32>> {
        Ok(None)
    }
    /// Issue 361: whether a recorded outcome was judged by the logic this
    /// build runs; every reuse and replay path asks before answering from it.
    fn outcome_logic_holds(&self, _record: &WorkflowV2CallRecord) -> WorkflowResult<bool> {
        Ok(true)
    }
    /// Issue 361: the digest of the source a call of `request` is judged
    /// by, stamped into its outcome; `None` for an executor that hashes none.
    fn logic_digest(&self, _request: &HostCommandRequest) -> WorkflowResult<Option<String>> {
        Ok(None)
    }
    /// Issue 361: the build a call of `request` is judged by, stamped into
    /// its outcome; `None` for an executor that names none.
    fn logic_build(&self, _request: &HostCommandRequest) -> WorkflowResult<Option<String>> {
        Ok(None)
    }

    async fn execute(
        &self,
        request: HostCommandRequest,
        expected_generation: Option<u64>,
    ) -> WorkflowResult<HostCommandResult>;
}
