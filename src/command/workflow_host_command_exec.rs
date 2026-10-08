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
    CommandCapabilityCatalog, GateEnvelopeV1, HostCommandRequest, HostCommandResult,
    PreparedPublicationV1, WorkflowError, WorkflowResult, WorkflowV2CallRecord,
    host_command_call_id,
};
use async_trait::async_trait;

use super::workflow_host_command_catalog::{
    HostCommandResolutionContext, ResolvedHostCommand, host_command_identity_tokens,
    resolve_host_command,
};
use super::workflow_host_command_decision::{
    candidate_findings_prevent_publication, candidate_refusal_envelope,
    candidate_refused_before_staging, unpublished,
};
use super::workflow_host_command_postcondition::{
    evaluate_postcondition, fixed_subject_is_terminal, receipt_matches_live,
};
use super::workflow_host_command_publish::{
    LiveMutationSentinels, audit_prepared_publication, prepare_staging, publish_audited,
};
use super::workflow_host_command_supervisor::{
    HostCommandControl, SupervisedProcessOutput, supervise_process_group,
};
use super::workflow_host_envelope_seal::{
    ENVELOPE_FILE, EnvelopeCleanup, owner_only, seal_staged_envelope,
};
use super::workflow_host_secrets::{HostSecrets, utf8};

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
        }
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
}

#[path = "workflow_host_command_exec_destinations.rs"]
mod destinations;
#[path = "workflow_host_command_exec_live.rs"]
mod live;
#[path = "workflow_host_command_exec_retry.rs"]
mod retry;

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

    fn judged_inputs(&self, request: &HostCommandRequest) -> WorkflowResult<Option<String>> {
        self.judged_inputs_for(request)
    }

    fn record_is_live(&self, record: &WorkflowV2CallRecord) -> WorkflowResult<bool> {
        self.record_is_reusable_live(record, true)
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
        Ok(self
            .logic_digests
            .get(&request.command_id)
            .map(|(digest, _)| digest.clone()))
    }

    fn logic_build(&self, _request: &HostCommandRequest) -> WorkflowResult<Option<String>> {
        Ok(Some(self.build.clone()))
    }

    async fn execute(
        &self,
        request: HostCommandRequest,
        expected_generation: Option<u64>,
    ) -> WorkflowResult<HostCommandResult> {
        let expected_generation = expected_generation.ok_or_else(|| {
            WorkflowError::StateCorrupt(
                "fixed HostCommand execution has no dispatch generation".into(),
            )
        })?;
        crate::command::workflow_host_command_integrity::require_launch_prd_unchanged(
            &self.context,
            &request.command_id,
        )?;
        // A body the host cannot bind to a frozen subject is the author's
        // artifact being wrong, not the host failing: it belongs in the findings
        // channel that gives the author its next attempt, exactly like a
        // candidate the gate refuses.
        let context = match self.context_for_request(&request) {
            Ok(context) => context,
            Err(WorkflowError::SpecInvalid(reason)) => {
                return Ok(unpublished(
                    Some(0),
                    String::new(),
                    String::new(),
                    (0, 0),
                    candidate_refusal_envelope(&request.command_id, &reason),
                    "candidate refused before staging",
                ));
            }
            Err(error) => return Err(error),
        };
        let call_id = self.content_identity(&request, &context)?;
        let staging = prepare_staging(&self.run_root, &call_id)
            .map_err(|error| WorkflowError::StageFailed(error.to_string()))?;
        let command = self.resolved(&request, &context, &call_id)?;
        if command
            .declared_write_set
            .iter()
            .any(|path| !path.starts_with(&staging.root))
        {
            return Err(WorkflowError::SpecInvalid(
                "resolved host command write set escaped prepared staging root".to_string(),
            ));
        }
        let destinations = self.destinations(&context, &command, &call_id)?;
        let sentinels =
            LiveMutationSentinels::capture(&destinations.values().cloned().collect::<Vec<_>>())
                .map_err(|error| WorkflowError::StageFailed(error.to_string()))?;
        // What the call records never holds a secret value the child was given.
        let secrets = HostSecrets::of(&context, &command.environment);
        let staged_envelope = staging.root.join(ENVELOPE_FILE);
        let _cleanup = EnvelopeCleanup {
            path: &staged_envelope,
            secrets: &secrets,
        };
        // Issue #255: an operational ending is retried or pauses the run.
        let observed = self
            .execute_with_operational_retry(&command, &call_id, expected_generation)
            .await?;
        let truncated = observed.truncation();
        let raw_stdout = utf8(observed.stdout, "stdout")?;
        let mut prepared: Option<PreparedPublicationV1> = if observed.exit_code == Some(0) {
            Some(secrets.parse_json(
                raw_stdout.trim().as_bytes(),
                &format!(
                    "host command '{}' returned malformed prepared manifest",
                    request.command_id,
                ),
            )?)
        } else {
            None
        };
        // Verify the manifest against raw bytes before any identity is sealed.
        seal_staged_envelope(&staged_envelope, &secrets, prepared.as_mut())?;
        let (stdout, stderr) = (
            secrets.text(&raw_stdout),
            secrets.text(&utf8(observed.stderr, "stderr")?),
        );
        if observed.exit_code != Some(0) {
            return Ok(HostCommandResult {
                exit_code: observed.exit_code,
                stdout,
                stderr,
                stdout_bytes: observed.stdout_bytes,
                stderr_bytes: observed.stderr_bytes,
                timed_out: observed.timed_out,
                interrupted: false,
                stdout_truncated: truncated.0,
                stderr_truncated: truncated.1,
                gate_envelope: None,
                publication_receipt: None,
                subjects: Vec::new(),
                postcondition: None,
            });
        }
        let Some(prepared) = prepared else {
            return Err(WorkflowError::StateCorrupt(
                "successful host call has no manifest".into(),
            ));
        };
        let envelope: GateEnvelopeV1 = secrets.parse_json(
            &std::fs::read(&staged_envelope).map_err(|source| WorkflowError::Io {
                path: staged_envelope.clone(),
                source,
            })?,
            "host command returned malformed gate envelope",
        )?;
        let envelope = secrets.envelope(envelope);
        if envelope.policy_findings.iter().any(|finding| {
            !command
                .remediation_scopes
                .contains(&finding.remediation_scope)
        }) {
            return Err(WorkflowError::PolicyDenied(format!(
                "host command '{}' returned a remediation scope outside its catalog",
                request.command_id
            )));
        }
        if envelope.operational_error.is_some() {
            // The script stops on `operational_error` itself, and it can only do
            // that if the envelope reaches it. Failing the call here instead
            // discarded the envelope and left the bridge with an error it could
            // not render as an outcome, so the operator saw a conversion
            // complaint rather than the reason the phase stopped.
            return Ok(unpublished(
                observed.exit_code,
                stdout,
                stderr,
                (observed.stdout_bytes, observed.stderr_bytes),
                envelope,
                "host gate reported an operational failure",
            ));
        }
        if candidate_refused_before_staging(&prepared, &command) {
            return Ok(unpublished(
                observed.exit_code,
                stdout,
                stderr,
                (observed.stdout_bytes, observed.stderr_bytes),
                envelope,
                "candidate refused before staging",
            ));
        }
        if candidate_findings_prevent_publication(&command.command_id, context.gate_mode, &envelope)
        {
            return Ok(unpublished(
                observed.exit_code,
                stdout,
                stderr,
                (observed.stdout_bytes, observed.stderr_bytes),
                envelope,
                "candidate findings prevented parent publication",
            ));
        }
        let store = archon_workflow::WorkflowStore::project(&self.context.project_root);
        let run_id = self
            .run_root
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                WorkflowError::StateCorrupt("fixed HostCommand run root has no UTF-8 run id".into())
            })?
            .to_string();
        let shadow_root = self.context.project_root.clone();
        let pin = super::workflow_task_set::acceptance_pin_path(
            &context.project_root,
            &context.task_root,
        );
        let (receipt, subjects, postcondition) = store.with_run_lock(&run_id, |locked| {
            // A sibling stopped by a pause reports "paused", not "cancelled".
            crate::command::workflow_host_command_operational::require_run_owned_locked(
                locked,
                &run_id,
                expected_generation,
            )?;
            let audited = audit_prepared_publication(&staging, &prepared, &command, sentinels)
                .map_err(|error| WorkflowError::StageFailed(error.to_string()))?;
            let receipt = publish_audited(audited, &destinations, &pin, &context.task_root)
                .map_err(|error| WorkflowError::StageFailed(error.to_string()))?;
            if let Some(envelope) = destinations.get(ENVELOPE_FILE) {
                owner_only(envelope)?;
            }
            // Parent-only recording follows every publication refusal.
            // Never fatal here. The publication is already committed to the
            // live tree; failing the call now would lose the receipt the script
            // needs while leaving the commit in place - the partial state this
            // whole path exists to prevent.
            if let Err(error) = crate::command::workflow_gate::append_published_shadow_records(
                &shadow_root,
                &call_id,
                &command.command_id,
                &envelope.policy_findings,
                "staged",
            ) {
                tracing::warn!(%error, "recording published gate findings failed");
            }
            let (subjects, postcondition) = evaluate_postcondition(&context, &command.command_id)?;
            Ok((receipt, subjects, postcondition))
        })?;
        Ok(HostCommandResult {
            exit_code: observed.exit_code,
            stdout,
            stderr,
            stdout_bytes: observed.stdout_bytes,
            stderr_bytes: observed.stderr_bytes,
            timed_out: false,
            interrupted: false,
            stdout_truncated: truncated.0,
            stderr_truncated: truncated.1,
            gate_envelope: Some(envelope),
            publication_receipt: Some(receipt),
            subjects,
            postcondition: Some(postcondition),
        })
    }
}

#[path = "workflow_host_command_exec_identity.rs"]
pub(crate) mod identity;
