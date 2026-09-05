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
    CommandCapabilityCatalog, CommandPostconditionEvaluation, GateEnvelopeV1, HostCommandRequest,
    HostCommandResult, PreparedPublicationV1, WorkflowError, WorkflowResult, WorkflowV2CallRecord,
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
    evaluate_postcondition, fixed_subject_is_terminal, read_acceptance_pin, receipt_matches_live,
};
use super::workflow_host_command_publish::{
    LiveMutationSentinels, audit_prepared_publication, prepare_staging, publish_audited,
};
use super::workflow_host_command_supervisor::{
    HostCommandControl, HostCommandControlHandle, HostCommandSignal, SupervisedProcessOutput,
    supervise_process_group,
};

#[async_trait]
pub(crate) trait WorkflowHostCommandExecutor: Send + Sync {
    fn call_identity(&self, request: &HostCommandRequest) -> WorkflowResult<String>;

    fn record_is_reusable(&self, record: &WorkflowV2CallRecord) -> WorkflowResult<bool>;
    /// Whether the record's landed outcome is still exactly what is on disk,
    /// findings or not: identity, receipt, postcondition and terminal subject.
    /// Defaults to the stricter reuse test.
    fn record_is_live(&self, record: &WorkflowV2CallRecord) -> WorkflowResult<bool> {
        self.record_is_reusable(record)
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

#[derive(Debug)]
pub(crate) struct DirectHostCommandProcessAdapter;

#[async_trait]
impl HostCommandProcessAdapter for DirectHostCommandProcessAdapter {
    async fn execute(
        &self,
        request: ResolvedHostCommand,
        control: HostCommandControl,
    ) -> WorkflowResult<SupervisedProcessOutput> {
        supervise_process_group(request, control).await
    }
}

pub(crate) struct FixedHostCommandExecutor {
    catalog: CommandCapabilityCatalog,
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
        Self::with_process(
            catalog,
            context,
            run_root,
            Arc::new(DirectHostCommandProcessAdapter),
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

    fn destinations(
        &self,
        context: &HostCommandResolutionContext,
        command: &ResolvedHostCommand,
        call_id: &str,
    ) -> WorkflowResult<BTreeMap<String, PathBuf>> {
        let envelope = self
            .run_root
            .join("host-command-results")
            .join(call_id)
            .join("gate-envelope.json");
        let mut destinations = BTreeMap::new();
        for path in &command.declared_write_set {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| {
                    WorkflowError::SpecInvalid(format!(
                        "declared host output {} has no UTF-8 file name",
                        path.display()
                    ))
                })?;
            let destination = match name {
                "gate-envelope.json" => envelope.clone(),
                "acceptance-contract.json"
                | "acceptance-contract.lock"
                | "task-skeleton.json"
                | "task-skeleton.lock" => context.task_root.join(name),
                "acceptance-pin.json" => super::workflow_task_set::acceptance_pin_path(
                    &context.project_root,
                    &context.task_root,
                ),
                _ if command.command_id == "land-task-body" => {
                    context.frozen_task_file.clone().ok_or_else(|| {
                        WorkflowError::SpecInvalid(
                            "land-task-body has no host-bound frozen task file".to_string(),
                        )
                    })?
                }
                _ => {
                    return Err(WorkflowError::SpecInvalid(format!(
                        "host command '{}' declared unknown output {name}",
                        command.command_id
                    )));
                }
            };
            if destinations.insert(name.to_string(), destination).is_some() {
                return Err(WorkflowError::SpecInvalid(format!(
                    "host command '{}' declared duplicate output {name}",
                    command.command_id
                )));
            }
        }
        Ok(destinations)
    }
    async fn execute_process_with_run_control(
        &self,
        request: ResolvedHostCommand,
        control: HostCommandControl,
        handle: HostCommandControlHandle,
        expected_generation: u64,
    ) -> WorkflowResult<SupervisedProcessOutput> {
        let store = archon_workflow::WorkflowStore::project(&self.context.project_root);
        let run_id = self
            .run_root
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                WorkflowError::StateCorrupt(
                    "fixed HostCommand run root has no UTF-8 run id".to_string(),
                )
            })?
            .to_string();
        let current_generation = store.load_state(&run_id)?.generation;
        if current_generation != expected_generation {
            return Err(WorkflowError::ControlCancelled(format!(
                "fixed HostCommand generation {expected_generation} no longer owns run {run_id}; current generation is {current_generation}"
            )));
        }
        let work = self.process.execute(request, control);
        tokio::pin!(work);
        let mut poll = tokio::time::interval(std::time::Duration::from_millis(100));
        loop {
            tokio::select! {
                biased;
                result = &mut work => return result,
                _ = poll.tick() => {
                    let Ok(run) = store.load_state(&run_id) else {
                        continue;
                    };
                    let signal = match run.status {
                        archon_workflow::RunStatus::Paused => Some(HostCommandSignal::Paused),
                        archon_workflow::RunStatus::Cancelled => Some(HostCommandSignal::Cancelled),
                        _ if run.generation != expected_generation =>
                        {
                            Some(HostCommandSignal::Cancelled)
                        }
                        _ => None,
                    };
                    if let Some(signal) = signal {
                        handle.signal(signal)?;
                        return work.await;
                    }
                }
            }
        }
    }
}

#[path = "workflow_host_command_exec_live.rs"]
mod live;

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
        Ok(host_command_call_id(
            &request.command_id,
            &self.catalog.digest,
            &self.catalog.starting_binary_revision,
            &host_command_identity_tokens(&context, &request.command_id)?,
            request.stdin.as_deref().unwrap_or_default().as_bytes(),
        ))
    }

    fn record_is_reusable(&self, record: &WorkflowV2CallRecord) -> WorkflowResult<bool> {
        self.record_is_reusable_live(record, false)
    }

    fn record_is_live(&self, record: &WorkflowV2CallRecord) -> WorkflowResult<bool> {
        self.record_is_reusable_live(record, true)
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
        let call_id = host_command_call_id(
            &request.command_id,
            &self.catalog.digest,
            &self.catalog.starting_binary_revision,
            &host_command_identity_tokens(&context, &request.command_id)?,
            request.stdin.as_deref().unwrap_or_default().as_bytes(),
        );
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
        let (control, handle) = HostCommandControl::new();
        let observed = self
            .execute_process_with_run_control(command.clone(), control, handle, expected_generation)
            .await?;
        let stdout = String::from_utf8(observed.stdout).map_err(|error| {
            WorkflowError::StageFailed(format!("host command stdout is not UTF-8: {error}"))
        })?;
        let stderr = String::from_utf8(observed.stderr).map_err(|error| {
            WorkflowError::StageFailed(format!("host command stderr is not UTF-8: {error}"))
        })?;
        if observed.exit_code != Some(0) {
            return Ok(HostCommandResult {
                exit_code: observed.exit_code,
                stdout,
                stderr,
                stdout_bytes: observed.stdout_bytes,
                stderr_bytes: observed.stderr_bytes,
                timed_out: false,
                interrupted: false,
                stdout_truncated: false,
                stderr_truncated: false,
                gate_envelope: None,
                publication_receipt: None,
                subjects: Vec::new(),
                postcondition: None,
            });
        }
        let prepared: PreparedPublicationV1 =
            serde_json::from_str(stdout.trim()).map_err(|error| {
                WorkflowError::StageFailed(format!(
                    "host command '{}' returned malformed prepared manifest: {error}",
                    request.command_id
                ))
            })?;
        let envelope_path = staging.root.join("gate-envelope.json");
        let envelope: GateEnvelopeV1 =
            serde_json::from_slice(&std::fs::read(&envelope_path).map_err(|error| {
                WorkflowError::Io {
                    path: envelope_path.clone(),
                    source: error,
                }
            })?)?;
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
        let (receipt, subjects, postcondition) = store.with_run_lock(&run_id, |locked| {
            let current = locked.load_state(&run_id)?;
            if current.generation != expected_generation {
                return Err(WorkflowError::ControlCancelled(format!(
                    "fixed HostCommand generation {expected_generation} cannot publish to run {run_id}; current generation is {}",
                    current.generation
                )));
            }
            let audited = audit_prepared_publication(&staging, &prepared, &command, sentinels)
                .map_err(|error| WorkflowError::StageFailed(error.to_string()))?;
            let receipt = publish_audited(audited, &destinations)
                .map_err(|error| WorkflowError::StageFailed(error.to_string()))?;
            // Only now, past every refusal the parent can still make. The
            // staged child cannot write here: a record appended before this
            // point survives a publication the parent rejects.
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
            let (subjects, postcondition) =
                evaluate_postcondition(&context, &command.command_id)?;
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
            stdout_truncated: false,
            stderr_truncated: false,
            gate_envelope: Some(envelope),
            publication_receipt: Some(receipt),
            subjects,
            postcondition: Some(postcondition),
        })
    }
}
