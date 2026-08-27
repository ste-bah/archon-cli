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
use super::workflow_host_command_postcondition::{
    evaluate_postcondition, fixed_subject_is_terminal, read_acceptance_pin, receipt_matches_live,
};
use super::workflow_host_command_publish::{
    LiveMutationSentinels, audit_prepared_publication, prepare_staging, publish_audited,
};
use super::workflow_host_command_supervisor::{
    HostCommandControl, SupervisedProcessOutput, supervise_process_group,
};

#[async_trait]
pub(crate) trait WorkflowHostCommandExecutor: Send + Sync {
    fn call_identity(&self, request: &HostCommandRequest) -> WorkflowResult<String>;

    fn record_is_reusable(&self, record: &WorkflowV2CallRecord) -> WorkflowResult<bool>;

    async fn execute(&self, request: HostCommandRequest) -> WorkflowResult<HostCommandResult>;
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

    fn context_for_request(
        &self,
        request: &HostCommandRequest,
    ) -> WorkflowResult<HostCommandResolutionContext> {
        let mut context = self.context.clone();
        context.run_staging_root = self.run_root.join("host-command-staging");
        if request.command_id != "land-task-body"
            || (context.frozen_task_id.is_some() && context.frozen_task_file.is_some())
        {
            return Ok(context);
        }
        let candidate = request.stdin.as_deref().ok_or_else(|| {
            WorkflowError::SpecInvalid("land-task-body requires candidate stdin".to_string())
        })?;
        let pin = read_acceptance_pin(&context)?;
        let skeleton =
            archon_workflow::task_skeleton::validate_full_chain(&context.task_root, &pin)
                .map_err(|error| WorkflowError::SpecInvalid(error.to_string()))?;
        let mut matches = Vec::new();
        for frozen in &skeleton.tasks {
            let path = context.task_root.join(&frozen.file_name);
            if archon_workflow::task_universe::parsing::parse_task_file(&path, candidate).is_ok() {
                matches.push((frozen.task_id.clone(), path));
            }
        }
        if matches.len() != 1 {
            return Err(WorkflowError::SpecInvalid(format!(
                "candidate TASK body binds {} frozen subjects; return exactly one body preserving a frozen task_id and file_name",
                matches.len()
            )));
        }
        let (task_id, task_file) = matches.pop().expect("one candidate subject");
        context.frozen_task_id = Some(task_id);
        context.frozen_task_file = Some(task_file);
        Ok(context)
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
                    self.context.frozen_task_file.clone().ok_or_else(|| {
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
}

#[async_trait]
impl WorkflowHostCommandExecutor for FixedHostCommandExecutor {
    fn call_identity(&self, request: &HostCommandRequest) -> WorkflowResult<String> {
        if !self.catalog.capabilities.contains_key(&request.command_id) {
            return Err(WorkflowError::SpecInvalid(format!(
                "undeclared host command capability '{}'",
                request.command_id
            )));
        }
        let context = self.context_for_request(request)?;
        Ok(host_command_call_id(
            &request.command_id,
            &self.catalog.digest,
            &self.catalog.starting_binary_revision,
            &host_command_identity_tokens(&context),
            request.stdin.as_deref().unwrap_or_default().as_bytes(),
        ))
    }

    fn record_is_reusable(&self, record: &WorkflowV2CallRecord) -> WorkflowResult<bool> {
        if record.call.method != archon_workflow::WorkflowV2HostMethod::HostCommand
            || !matches!(
                record.status,
                archon_workflow::WorkflowV2Status::Accepted
                    | archon_workflow::WorkflowV2Status::Noop
            )
            || record.invalidated_by.is_some()
        {
            return Ok(false);
        }
        let request = record.call.options.host_command.as_ref().ok_or_else(|| {
            WorkflowError::StateCorrupt("persisted HostCommand record has no typed request".into())
        })?;
        if self.call_identity(request)? != record.call.id {
            return Ok(false);
        }
        let outcome: HostCommandResult = serde_json::from_value(record.result.data.clone())?;
        if !outcome.reusable() || !receipt_matches_live(outcome.publication_receipt.as_ref())? {
            return Ok(false);
        }
        let context = self.context_for_request(request)?;
        let (_, current_postcondition) = evaluate_postcondition(&context, &request.command_id)?;
        if !current_postcondition.satisfied {
            return Ok(false);
        }
        fixed_subject_is_terminal(&self.run_root, &request.command_id, &outcome)
    }

    async fn execute(&self, request: HostCommandRequest) -> WorkflowResult<HostCommandResult> {
        let context = self.context_for_request(&request)?;
        let call_id = host_command_call_id(
            &request.command_id,
            &self.catalog.digest,
            &self.catalog.starting_binary_revision,
            &host_command_identity_tokens(&context),
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
        let (control, _handle) = HostCommandControl::new();
        let observed = self.process.execute(command.clone(), control).await?;
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
        if let Some(error) = &envelope.operational_error {
            return Err(WorkflowError::StageFailed(error.text.clone()));
        }
        if candidate_findings_prevent_publication(&command.command_id, context.gate_mode, &envelope)
        {
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
                gate_envelope: Some(envelope),
                publication_receipt: None,
                subjects: Vec::new(),
                postcondition: Some(CommandPostconditionEvaluation {
                    satisfied: false,
                    summary: "candidate findings prevented parent publication".into(),
                }),
            });
        }
        let audited = audit_prepared_publication(&staging, &prepared, &command, sentinels)
            .map_err(|error| WorkflowError::StageFailed(error.to_string()))?;
        let receipt = publish_audited(audited, &destinations)
            .map_err(|error| WorkflowError::StageFailed(error.to_string()))?;
        let (subjects, postcondition) = evaluate_postcondition(&context, &command.command_id)?;
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

fn candidate_findings_prevent_publication(
    command_id: &str,
    mode: archon_core::config::GateMode,
    envelope: &GateEnvelopeV1,
) -> bool {
    use archon_workflow::RemediationScope;

    envelope.policy_findings.iter().any(|finding| {
        if matches!(
            finding.remediation_scope,
            RemediationScope::PrdInput | RemediationScope::Operational
        ) {
            return true;
        }
        if mode == archon_core::config::GateMode::Observe {
            return false;
        }
        match command_id {
            "freeze-acceptance" => finding.remediation_scope == RemediationScope::CandidateArtifact,
            "freeze-skeleton" => matches!(
                finding.remediation_scope,
                RemediationScope::CandidateArtifact | RemediationScope::Skeleton
            ),
            "land-task-body" => finding.remediation_scope != RemediationScope::InheritedPredecessor,
            _ => false,
        }
    })
}
