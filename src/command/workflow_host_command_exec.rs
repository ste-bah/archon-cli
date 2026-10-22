//! Concrete persisted HostCommand execution boundary.
//!
//! The script supplies only a symbolic capability and bounded stdin. This
//! module computes the stable identity, resolves host-owned process authority,
//! supervises the trusted child, audits its run-owned staging tree, and alone
//! publishes exact bytes to live destinations.
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use archon_workflow::{
    CommandCapabilityCatalog, GateEnvelopeV1, HostCommandRequest, HostCommandResult, WorkflowError,
    WorkflowResult, WorkflowV2CallRecord, host_command_call_id,
};
use async_trait::async_trait;

use super::workflow_host_command_catalog::{
    HostCommandResolutionContext, ResolvedHostCommand, host_command_identity_tokens,
    resolve_host_command,
};
use super::workflow_host_command_decision::{
    candidate_findings_prevent_publication, candidate_refusal_envelope,
    candidate_refused_before_staging, stale_acceptance_contract, unpublished,
};
use super::workflow_host_command_postcondition::{
    evaluate_postcondition, fixed_subject_is_terminal,
};
use super::workflow_host_command_publish::{
    LiveMutationSentinels, audit_prepared_publication, publish_audited,
};
use super::workflow_host_command_supervisor::{
    HostCommandControl, SupervisedProcessOutput, supervise_process_group,
};
use super::workflow_host_command_teardown_latch::TeardownLatch;
use super::workflow_host_envelope_seal::{ENVELOPE_FILE, EnvelopeCleanup, owner_only};
use super::workflow_host_secrets::{HostSecrets, utf8};

#[path = "workflow_host_command_exec_note.rs"]
mod note;
pub(super) use note::append_host_note;

#[path = "workflow_host_command_exec_trait.rs"]
mod executor_trait;
pub(crate) use executor_trait::LandingRejudgeOutcome;
pub(crate) use executor_trait::WorkflowHostCommandExecutor;

#[async_trait]
pub(crate) trait HostCommandProcessAdapter: Send + Sync {
    async fn execute(
        &self,
        request: ResolvedHostCommand,
        control: HostCommandControl,
    ) -> WorkflowResult<SupervisedProcessOutput>;
}

/// Runs host commands for real. `group_records` is where a run keeps the
/// records of its process groups in flight; `None` keeps none.
#[derive(Debug, Default)]
pub(crate) struct DirectHostCommandProcessAdapter {
    pub(crate) group_records: Option<PathBuf>,
}

#[async_trait]
impl HostCommandProcessAdapter for DirectHostCommandProcessAdapter {
    async fn execute(
        &self,
        request: ResolvedHostCommand,
        control: HostCommandControl,
    ) -> WorkflowResult<SupervisedProcessOutput> {
        supervise_process_group(request, control, self.group_records.as_deref()).await
    }
}

#[cfg(test)]
thread_local! {
    /// Runs on the call's staging after its work and before its final seal.
    pub(crate) static AFTER_CALL: std::cell::Cell<Option<Box<dyn FnOnce(&std::path::Path)>>> =
        const { std::cell::Cell::new(None) };
}

pub(crate) struct FixedHostCommandExecutor {
    catalog: CommandCapabilityCatalog,
    launch_catalog: Option<CommandCapabilityCatalog>,
    /// Issue 361: each capability's logic version, part of its reuse key.
    logic: BTreeMap<String, u32>,
    /// Issue 361: each capability's pinned logic digest, and whether its
    /// reuse binds to the build.
    logic_digests: BTreeMap<String, (String, bool)>,
    /// Issue 361: the build that judges every call here.
    build: String,
    context: HostCommandResolutionContext,
    run_root: PathBuf,
    process: Arc<dyn HostCommandProcessAdapter>,
    paused_run_authorization:
        Option<super::workflow_host_command_operational::PausedRunAuthorization>,
}

impl FixedHostCommandExecutor {
    pub(crate) fn new(
        catalog: CommandCapabilityCatalog,
        context: HostCommandResolutionContext,
        run_root: PathBuf,
    ) -> Self {
        let group_records =
            Some(run_root.join(super::workflow_host_command_groups::GROUP_RECORDS_DIR));
        Self::with_process(
            catalog,
            context,
            run_root,
            Arc::new(DirectHostCommandProcessAdapter { group_records }),
        )
    }

    pub(crate) fn with_process(
        catalog: CommandCapabilityCatalog,
        context: HostCommandResolutionContext,
        run_root: PathBuf,
        process: Arc<dyn HostCommandProcessAdapter>,
    ) -> Self {
        Self {
            catalog,
            launch_catalog: None,
            logic: super::workflow_host_command_logic::versions(),
            logic_digests: super::workflow_host_command_logic::digests(),
            build: super::workflow_host_command_logic::THIS_BUILD.to_string(),
            context,
            run_root,
            process,
            paused_run_authorization: None,
        }
    }

    pub(crate) fn authorize_paused_run(
        mut self,
        authorization: super::workflow_host_command_operational::PausedRunAuthorization,
    ) -> Self {
        self.paused_run_authorization = Some(authorization);
        self
    }

    fn unbound_context(&self) -> HostCommandResolutionContext {
        super::workflow_host_command_binding::unbound_context(&self.context, &self.run_root)
    }

    fn context_for_request(
        &self,
        request: &HostCommandRequest,
    ) -> WorkflowResult<HostCommandResolutionContext> {
        super::workflow_host_command_binding::context_for_request(
            &self.context,
            &self.run_root,
            request,
        )
    }

    fn resolved(
        &self,
        request: &HostCommandRequest,
        context: &HostCommandResolutionContext,
        call_id: &str,
    ) -> WorkflowResult<ResolvedHostCommand> {
        resolve_host_command(request, &self.catalog, context, call_id)
    }

    pub(crate) async fn execute_judge_only(
        &self,
        request: HostCommandRequest,
        expected_generation: Option<u64>,
    ) -> WorkflowResult<HostCommandResult> {
        if request.command_id != "land-task-body" {
            return Err(WorkflowError::SpecInvalid(
                "judge-only body execution requires land-task-body".into(),
            ));
        }
        self.execute_observed_only(request, expected_generation)
            .await
    }

    pub(crate) async fn execute_observed_only(
        &self,
        request: HostCommandRequest,
        expected_generation: Option<u64>,
    ) -> WorkflowResult<HostCommandResult> {
        self.execute_inner(request, expected_generation, None, true)
            .await
    }
}

#[path = "workflow_host_command_exec_destinations.rs"]
mod destinations;
#[path = "workflow_host_command_exec_finish.rs"]
pub(crate) mod finish;
#[path = "workflow_host_command_exec_live.rs"]
mod live;
pub(crate) use live::pipeline_stage;
#[path = "workflow_host_command_exec_retry.rs"]
mod retry;
#[path = "workflow_host_command_exec_stamp.rs"]
mod stamp;

#[async_trait]
impl WorkflowHostCommandExecutor for FixedHostCommandExecutor {
    fn call_identity(&self, request: &HostCommandRequest) -> WorkflowResult<String> {
        crate::command::workflow_host_command_integrity::require_launch_prd_unchanged(
            &self.context,
            &request.command_id,
        )?;
        if !self.catalog.capabilities.contains_key(&request.command_id) {
            return Err(WorkflowError::SpecInvalid(format!(
                "undeclared host command capability '{}'",
                request.command_id
            )));
        }
        // A candidate that binds no frozen subject still needs a call identity.
        // Refusing it is `execute`'s job, and it answers with a finding the
        // author can act on; failing here instead ends the run at dispatch,
        // before that refusal can ever be recorded.
        let context = match self.context_for_request(request) {
            Ok(context) => context,
            Err(WorkflowError::SpecInvalid(_)) => self.unbound_context(),
            Err(error) => return Err(error),
        };
        self.content_identity(request, &context)
    }

    fn record_is_reusable(&self, record: &WorkflowV2CallRecord) -> WorkflowResult<bool> {
        self.record_is_reusable_live(record, false)
    }

    fn publishes(&self, request: &HostCommandRequest) -> WorkflowResult<bool> {
        Ok(self.publishes_for(request))
    }
    fn judged_inputs(&self, request: &HostCommandRequest) -> WorkflowResult<Option<String>> {
        self.judged_inputs_for(request)
    }

    fn record_is_live(&self, record: &WorkflowV2CallRecord) -> WorkflowResult<bool> {
        self.record_is_reusable_live(record, true)
    }

    fn landing_receipt_matches_disk(&self, record: &WorkflowV2CallRecord) -> WorkflowResult<bool> {
        self.landing_receipt_matches_disk_live(record)
    }

    async fn rejudge_landing(
        &self,
        record: &WorkflowV2CallRecord,
        expected_generation: Option<u64>,
        allow_restore: bool,
    ) -> WorkflowResult<Option<LandingRejudgeOutcome>> {
        self.rejudge_landing_without_publication(record, expected_generation, allow_restore)
            .await
    }

    async fn repair_landing_mismatch(
        &self,
        record: &WorkflowV2CallRecord,
        expected_generation: Option<u64>,
        allow_restore: bool,
    ) -> WorkflowResult<(bool, Option<HostCommandResult>, bool)> {
        self.repair_landing_mismatch_live(record, expected_generation, allow_restore)
            .await
    }

    fn outcome_limits_hold(&self, record: &WorkflowV2CallRecord) -> WorkflowResult<bool> {
        self.outcome_limits_hold_for(record)
    }

    fn limits_fingerprint(
        &self,
        request: &HostCommandRequest,
    ) -> WorkflowResult<Option<serde_json::Value>> {
        self.limits_fingerprint_for(request)
    }

    fn logic_version(&self, request: &HostCommandRequest) -> WorkflowResult<Option<u32>> {
        self.logic_version_for(&request.command_id).map(Some)
    }

    fn outcome_logic_holds(&self, record: &WorkflowV2CallRecord) -> WorkflowResult<bool> {
        self.outcome_logic_holds_for(record)
    }

    fn logic_digest(&self, request: &HostCommandRequest) -> WorkflowResult<Option<String>> {
        Ok(self.logic_digest_for(request))
    }

    fn logic_build(&self, _request: &HostCommandRequest) -> WorkflowResult<Option<String>> {
        Ok(Some(self.build.clone()))
    }

    async fn execute(
        &self,
        request: HostCommandRequest,
        expected_generation: Option<u64>,
    ) -> WorkflowResult<HostCommandResult> {
        self.execute_inner(request, expected_generation, None, false)
            .await
    }
}
#[path = "workflow_host_command_exec_identity.rs"]
pub(crate) mod identity;
#[path = "workflow_host_command_exec_inner.rs"]
mod inner;
