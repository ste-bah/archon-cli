use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use super::workflow_host_command_paths::{
    validate_existing_path, validate_lexical_absolute, validate_publication_destination,
    validate_task_id,
};
use archon_workflow::{
    CommandCapability, CommandCapabilityCatalog, EnvironmentProfileId, HostCommandRequest,
    RemediationScope, StdinDelivery, WorkflowError, WorkflowResult,
};

const CATALOG_SCHEMA_VERSION: u32 = 1;
const MIB: u64 = 1024 * 1024;

#[derive(Debug, Clone)]
pub(crate) struct HostCommandResolutionContext {
    pub(crate) program: PathBuf,
    pub(crate) project_root: PathBuf,
    pub(crate) prd_path: PathBuf,
    pub(crate) prd_digest: String,
    pub(crate) task_root: PathBuf,
    pub(crate) run_staging_root: PathBuf,
    pub(crate) frozen_task_id: Option<String>,
    pub(crate) frozen_task_file: Option<PathBuf>,
    pub(crate) freeze_provider_environment: BTreeMap<String, String>,
    pub(crate) gate_mode: archon_core::config::GateMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedHostCommand {
    pub(crate) command_id: String,
    pub(crate) program: PathBuf,
    pub(crate) args: Vec<String>,
    pub(crate) cwd: PathBuf,
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) stdin: Option<Vec<u8>>,
    pub(crate) timeout_secs: u64,
    pub(crate) max_stdout_bytes: u64,
    pub(crate) max_stderr_bytes: u64,
    pub(crate) declared_write_set: Vec<PathBuf>,
    pub(crate) remediation_scopes: BTreeSet<RemediationScope>,
}

pub(crate) fn fixed_decomposition_catalog(
    starting_binary_revision: &str,
) -> WorkflowResult<CommandCapabilityCatalog> {
    let mut capabilities = BTreeMap::new();
    insert(
        &mut capabilities,
        capability(
            "freeze-acceptance",
            &[
                "workflow",
                "freeze-acceptance",
                "--tasks",
                "{TASK_ROOT}",
                "--prd",
                "{PRD_PATH}",
                "--candidate-stdin",
                "--call-id",
                "{CALL_ID}",
                "--staging-root",
                "{COMMAND_STAGING}",
                "--gate-envelope",
                "{GATE_ENVELOPE}",
            ],
            StdinDelivery::Utf8Bytes,
            EnvironmentProfileId::FreezeProvider,
            1_500,
            2 * MIB,
            2 * MIB,
            2 * MIB,
            &[
                "{COMMAND_STAGING}/acceptance-contract.json",
                "{COMMAND_STAGING}/acceptance-contract.lock",
                "{COMMAND_STAGING}/acceptance-pin.json",
                "{GATE_ENVELOPE}",
            ],
            &[
                RemediationScope::CandidateArtifact,
                RemediationScope::InheritedPredecessor,
                RemediationScope::PrdInput,
                RemediationScope::Operational,
            ],
        ),
    );
    insert(
        &mut capabilities,
        capability(
            "freeze-skeleton",
            &[
                "workflow",
                "freeze-skeleton",
                "--tasks",
                "{TASK_ROOT}",
                "--prd",
                "{PRD_PATH}",
                "--candidate-stdin",
                "--call-id",
                "{CALL_ID}",
                "--staging-root",
                "{COMMAND_STAGING}",
                "--gate-envelope",
                "{GATE_ENVELOPE}",
            ],
            StdinDelivery::Utf8Bytes,
            EnvironmentProfileId::None,
            1_500,
            2 * MIB,
            2 * MIB,
            2 * MIB,
            &[
                "{COMMAND_STAGING}/task-skeleton.json",
                "{COMMAND_STAGING}/task-skeleton.lock",
                "{COMMAND_STAGING}/acceptance-pin.json",
                "{GATE_ENVELOPE}",
            ],
            &[
                RemediationScope::CandidateArtifact,
                RemediationScope::Skeleton,
                RemediationScope::InheritedPredecessor,
                RemediationScope::PrdInput,
                RemediationScope::Operational,
            ],
        ),
    );
    insert(
        &mut capabilities,
        capability(
            "land-task-body",
            &[
                "workflow",
                "lint",
                "--task-file",
                "{FROZEN_TASK_FILE}",
                "--candidate-stdin",
                "--call-id",
                "{CALL_ID}",
                "--staging-root",
                "{COMMAND_STAGING}",
                "--gate-envelope",
                "{GATE_ENVELOPE}",
            ],
            StdinDelivery::Utf8Bytes,
            EnvironmentProfileId::None,
            300,
            MIB,
            MIB,
            MIB,
            &[
                "{COMMAND_STAGING}/{FROZEN_TASK_FILE_NAME}",
                "{GATE_ENVELOPE}",
            ],
            &[
                RemediationScope::Body,
                RemediationScope::InheritedPredecessor,
                RemediationScope::Skeleton,
                RemediationScope::PrdInput,
                RemediationScope::Operational,
            ],
        ),
    );
    insert(
        &mut capabilities,
        capability(
            "task-set-lint",
            &[
                "workflow",
                "lint",
                "--tasks",
                "{TASK_ROOT}",
                "--gate-envelope",
                "{GATE_ENVELOPE}",
                "--call-id",
                "{CALL_ID}",
            ],
            StdinDelivery::None,
            EnvironmentProfileId::None,
            300,
            0,
            4 * MIB,
            4 * MIB,
            &["{GATE_ENVELOPE}"],
            &[
                RemediationScope::Skeleton,
                RemediationScope::InheritedPredecessor,
                RemediationScope::PrdInput,
                RemediationScope::Operational,
            ],
        ),
    );
    insert(
        &mut capabilities,
        capability(
            "requirements-trace",
            &[
                "requirements",
                "trace",
                "--prd",
                "{PRD_PATH}",
                "--tasks",
                "{TASK_ROOT}",
                "--gate-envelope",
                "{GATE_ENVELOPE}",
                "--call-id",
                "{CALL_ID}",
            ],
            StdinDelivery::None,
            EnvironmentProfileId::None,
            300,
            0,
            4 * MIB,
            4 * MIB,
            &["{GATE_ENVELOPE}"],
            &[
                RemediationScope::Skeleton,
                RemediationScope::PrdInput,
                RemediationScope::Operational,
            ],
        ),
    );
    let mut catalog = CommandCapabilityCatalog {
        schema_version: CATALOG_SCHEMA_VERSION,
        starting_binary_revision: starting_binary_revision.to_string(),
        digest: String::new(),
        capabilities,
    };
    catalog.recompute_digest()?;
    Ok(catalog)
}

pub(crate) fn is_set_gate_command(command_id: &str) -> bool {
    matches!(command_id, "task-set-lint" | "requirements-trace")
}

pub(crate) fn host_command_identity_tokens(
    context: &HostCommandResolutionContext,
    command_id: &str,
) -> WorkflowResult<BTreeMap<String, String>> {
    let mut tokens = BTreeMap::from([
        (
            "PROJECT_ROOT".to_string(),
            context.project_root.to_string_lossy().into_owned(),
        ),
        (
            "PRD_PATH".to_string(),
            context.prd_path.to_string_lossy().into_owned(),
        ),
        ("PRD_DIGEST".to_string(), context.prd_digest.clone()),
        (
            "TASK_ROOT".to_string(),
            context.task_root.to_string_lossy().into_owned(),
        ),
    ]);
    if let Some(task_id) = &context.frozen_task_id {
        tokens.insert("FROZEN_TASK_ID".to_string(), task_id.clone());
    }
    if let Some(task_file) = &context.frozen_task_file {
        tokens.insert(
            "FROZEN_TASK_FILE".to_string(),
            task_file.to_string_lossy().into_owned(),
        );
    }
    if is_set_gate_command(command_id) {
        tokens.insert(
            "SET_GATE_INPUT_MANIFEST".to_string(),
            crate::command::workflow_host_command_manifest::set_gate_input_manifest_digest(
                context,
            )?,
        );
    }
    Ok(tokens)
}

pub(crate) fn resolve_host_command(
    request: &HostCommandRequest,
    catalog: &CommandCapabilityCatalog,
    context: &HostCommandResolutionContext,
    call_id: &str,
) -> WorkflowResult<ResolvedHostCommand> {
    let capability = catalog.capabilities.get(&request.command_id).ok_or_else(|| {
        WorkflowError::SpecInvalid(format!(
            "undeclared host command capability '{}'; fixed scripts may call only the persisted catalog",
            request.command_id
        ))
    })?;
    if capability.detaches {
        return Err(WorkflowError::PolicyDenied(format!(
            "host command capability '{}' may detach descendants and is ineligible",
            capability.id
        )));
    }
    validate_existing_path(&context.project_root, None, "project root")?;
    validate_existing_path(&context.prd_path, Some(&context.project_root), "PRD path")?;
    validate_existing_path(&context.task_root, Some(&context.project_root), "task root")?;
    validate_lexical_absolute(&context.run_staging_root, "run staging root")?;

    if call_id.trim().is_empty()
        || !call_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
    {
        return Err(WorkflowError::SpecInvalid(
            "host command call identity is empty or not a safe path component".to_string(),
        ));
    }
    let command_staging = context.run_staging_root.join(call_id);
    let gate_envelope = command_staging.join("gate-envelope.json");
    let mut tokens = BTreeMap::from([
        ("PROJECT_ROOT", context.project_root.clone()),
        ("PRD_PATH", context.prd_path.clone()),
        ("TASK_ROOT", context.task_root.clone()),
        ("COMMAND_STAGING", command_staging),
        ("GATE_ENVELOPE", gate_envelope),
        ("CALL_ID", PathBuf::from(call_id)),
    ]);
    if let Some(task_id) = &context.frozen_task_id {
        validate_task_id(task_id)?;
        tokens.insert("FROZEN_TASK_ID", PathBuf::from(task_id));
    }
    if let Some(task_file) = &context.frozen_task_file {
        // The frozen task file is where this body will be published, and the
        // first body for a task creates it. Requiring it to exist already made
        // the very first publication impossible, so the check is confinement and
        // shape — inside the task root, no symlink — not existence.
        validate_publication_destination(task_file, &context.task_root, "frozen task file")?;
        let parent = task_file.parent().ok_or_else(|| {
            WorkflowError::SpecInvalid("frozen task file has no parent".to_string())
        })?;
        if parent != context.task_root {
            return Err(WorkflowError::SpecInvalid(format!(
                "frozen task file {} is not a direct child of task root {}",
                task_file.display(),
                context.task_root.display()
            )));
        }
        let name = task_file.file_name().ok_or_else(|| {
            WorkflowError::SpecInvalid("frozen task file has no file name".to_string())
        })?;
        tokens.insert("FROZEN_TASK_FILE", task_file.clone());
        tokens.insert("FROZEN_TASK_FILE_NAME", PathBuf::from(name));
    }

    let args = capability
        .argv_template
        .iter()
        .map(|part| resolve_template(part, &tokens))
        .collect::<WorkflowResult<Vec<_>>>()?;
    let declared_write_set = capability
        .declared_write_set
        .iter()
        .map(|part| resolve_template(part, &tokens).map(PathBuf::from))
        .collect::<WorkflowResult<Vec<_>>>()?;
    let stdin = match (&capability.stdin_delivery, &request.stdin) {
        (StdinDelivery::None, Some(_)) => {
            return Err(WorkflowError::SpecInvalid(format!(
                "host command capability '{}' does not accept stdin",
                capability.id
            )));
        }
        (StdinDelivery::None, None) => None,
        (StdinDelivery::Utf8Bytes, None) => {
            return Err(WorkflowError::SpecInvalid(format!(
                "host command capability '{}' requires candidate stdin",
                capability.id
            )));
        }
        (StdinDelivery::Utf8Bytes, Some(value)) => {
            if value.len() as u64 > capability.max_stdin_bytes {
                return Err(WorkflowError::SpecInvalid(format!(
                    "host command capability '{}' stdin exceeds {} bytes",
                    capability.id, capability.max_stdin_bytes
                )));
            }
            Some(value.as_bytes().to_vec())
        }
    };
    let environment = match capability.environment_profile {
        EnvironmentProfileId::None => BTreeMap::new(),
        EnvironmentProfileId::FreezeProvider => context.freeze_provider_environment.clone(),
    };
    Ok(ResolvedHostCommand {
        command_id: capability.id.clone(),
        program: context.program.clone(),
        args,
        cwd: context.project_root.clone(),
        environment,
        stdin,
        timeout_secs: capability.timeout_secs,
        max_stdout_bytes: capability.max_stdout_bytes,
        max_stderr_bytes: capability.max_stderr_bytes,
        declared_write_set,
        remediation_scopes: capability.remediation_scopes.clone(),
    })
}

#[allow(clippy::too_many_arguments)]
fn capability(
    id: &str,
    argv_template: &[&str],
    stdin_delivery: StdinDelivery,
    environment_profile: EnvironmentProfileId,
    timeout_secs: u64,
    max_stdin_bytes: u64,
    max_stdout_bytes: u64,
    max_stderr_bytes: u64,
    declared_write_set: &[&str],
    remediation_scopes: &[RemediationScope],
) -> CommandCapability {
    CommandCapability {
        id: id.to_string(),
        argv_template: argv_template
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
        stdin_delivery,
        environment_profile,
        timeout_secs,
        max_stdin_bytes,
        max_stdout_bytes,
        max_stderr_bytes,
        declared_write_set: declared_write_set
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
        remediation_scopes: remediation_scopes.iter().copied().collect(),
        detaches: false,
    }
}

fn insert(capabilities: &mut BTreeMap<String, CommandCapability>, capability: CommandCapability) {
    let id = capability.id.clone();
    assert!(capabilities.insert(id, capability).is_none());
}

fn resolve_template(part: &str, tokens: &BTreeMap<&str, PathBuf>) -> WorkflowResult<String> {
    let mut resolved = part.to_string();
    for (name, value) in tokens {
        resolved = resolved.replace(&format!("{{{name}}}"), &value.to_string_lossy());
    }
    if resolved.contains('{') || resolved.contains('}') {
        return Err(WorkflowError::SpecInvalid(format!(
            "host command template contains an unresolved token: {resolved}"
        )));
    }
    Ok(resolved)
}
