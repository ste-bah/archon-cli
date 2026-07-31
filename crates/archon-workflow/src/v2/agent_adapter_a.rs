pub use super::agent_repair::WorkflowV2AgentError;
use super::project_artifact_completion::enforce_declared_artifact_requirements;
use super::{
    WorkflowV2CommandKind, WorkflowV2CommandStatus, WorkflowV2EvidenceKind, WorkflowV2HostCall,
    WorkflowV2ProjectArtifactContext, WorkflowV2Result, WorkflowV2Status,
    WorkflowV2TaskCoverageStatus, WorkflowV2WriteItem, WorkflowV2WriteMode,
    has_project_artifact_evidence, has_project_artifact_requirement,
    normalize_project_artifact_files, normalize_target_for_repository,
    normalize_targets_for_repository, validate_changed_files,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowV2AgentRequest {
    pub call: WorkflowV2HostCall,
    pub role: String,
    pub task: String,
    #[serde(default)]
    pub constraints: Vec<String>,
    #[serde(default)]
    pub input: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository_root: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "WorkflowV2ProjectArtifactContext::is_empty"
    )]
    pub project_artifacts: WorkflowV2ProjectArtifactContext,
    #[serde(default)]
    pub target_files: Vec<String>,
    #[serde(default)]
    pub target_ownership_scopes: Vec<String>,
}

impl WorkflowV2AgentRequest {
    /// Write capability is DECLARED via the call's write mode, never inferred
    /// from role names. Read-only calls that borrow a coder-tier model (e.g.
    /// focused verification) must not be held to implementation contracts
    /// they were instructed not to satisfy.
    pub fn is_write_capable(&self) -> bool {
        self.call.write_mode.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowV2PromptParts {
    pub stable_prefix: String,
    pub invocation: String,
}

#[derive(Debug, Clone, Default)]
pub struct WorkflowV2AgentAdapter;

impl WorkflowV2AgentAdapter {
    pub fn new() -> Self {
        Self
    }
    pub fn build_prompt(&self, request: &WorkflowV2AgentRequest) -> String {
        let parts = self.build_prompt_parts(request);
        format!("{}\n\n{}", parts.invocation, parts.stable_prefix)
    }

    pub fn build_prompt_parts(&self, request: &WorkflowV2AgentRequest) -> WorkflowV2PromptParts {
        super::agent_prompt::build_prompt_parts(request)
    }
    pub fn build_repair_prompt(
        &self,
        request: &WorkflowV2AgentRequest,
        invalid_output: &str,
        error: &WorkflowV2AgentError,
    ) -> String {
        let target_files = serde_json::to_string(&request.target_files).unwrap_or_default();
        let target_scopes =
            serde_json::to_string(&request.target_ownership_scopes).unwrap_or_default();
        let final_output_rule = if request.is_write_capable() {
            FINAL_OUTPUT_RULE
        } else {
            ""
        };
        format!(
            "The previous workflow V2 agent response for call '{}' was invalid.\n\n\
             Error: {error}\n\n\
             Return exactly one JSON object matching the required result envelope. \
             Do not include markdown fences, restored-context summaries, confirmation questions, \
             provider names, model names, or plan-only text.\n\n\
             Declared target_files: {target_files}\n\
             Declared target ownership scopes: {target_scopes}\n\
             Do not edit or claim repository files outside that ownership.\n\n\
             Task:\n{}\n\n\
             Required JSON Result Envelope:\n{RESULT_SCHEMA}\n\n\
             Previous invalid output excerpt:\n{}\n\n\
             {final_output_rule}\n",
            request.call.id,
            request.task,
            truncate_chars(invalid_output, 2_000),
        )
    }

    pub fn parse_agent_output(
        &self,
        request: &WorkflowV2AgentRequest,
        output: &str,
    ) -> Result<WorkflowV2Result, WorkflowV2AgentError> {
        reject_forbidden_text(output)?;
        let value = super::agent_output_normalize::normalize_agent_output(request, output)
            .map_err(|err| {
                WorkflowV2AgentError::MalformedOutput(format!(
                    "agent output must be one JSON WorkflowV2Result object: {err}; output begins: {}",
                    output_excerpt(output)
                ))
            })?;
        let mut result: WorkflowV2Result = serde_json::from_value(value).map_err(|err| {
            WorkflowV2AgentError::MalformedOutput(format!(
                "agent output must be one JSON WorkflowV2Result object: {err}; output begins: {}",
                output_excerpt(output)
            ))
        })?;
        self.validate_agent_result(request, &mut result)?;
        Ok(result)
    }

    fn validate_agent_result(
        &self,
        request: &WorkflowV2AgentRequest,
        result: &mut WorkflowV2Result,
    ) -> Result<(), WorkflowV2AgentError> {
        normalize_read_only_test_inspection(request, result);
        if request.is_write_capable() {
            enforce_declared_artifact_requirements(
                &request.call.id,
                &request.input,
                &request.call.options.required_artifacts,
                result,
                &request.project_artifacts,
            );
        }
        result.validate().map_err(|err| {
            WorkflowV2AgentError::InvalidResult(format!("agent result failed validation: {err}"))
        })?;
        validate_request_specific_result(request, result)
    }
}

#[async_trait::async_trait]
pub trait WorkflowV2AgentClient {
    async fn run_agent_request(
        &self,
        _request: &WorkflowV2AgentRequest,
        prompt: String,
    ) -> Result<String, WorkflowV2AgentError> {
        self.run_agent(prompt).await
    }

    async fn run_agent(&self, prompt: String) -> Result<String, WorkflowV2AgentError>;
}

fn validate_request_specific_result(
    request: &WorkflowV2AgentRequest,
    result: &mut WorkflowV2Result,
) -> Result<(), WorkflowV2AgentError> {
    reject_forbidden_result_text(result)?;
    if !request.is_write_capable() {
        if !result.files_changed.is_empty() {
            return Err(WorkflowV2AgentError::ReadOnlyChangedFiles);
        }
        return Ok(());
    }
    if plan_only_text(result) {
        return Err(WorkflowV2AgentError::PlanOnlyImplementation);
    }
    normalize_project_artifact_files(&request.call.id, result, &request.project_artifacts)
        .map_err(|err| {
            WorkflowV2AgentError::ImplementationChangedFilesOutsideOwnership(err.to_string())
        })?;
    validate_write_ownership(request, result)?;
    // A task that declares required_tools must actually EXERCISE each of them.
    // The no-op guard below already forbids skipping them via a no-op, but an
    // accepted result can also silently drop a required tool — running only the
    // easy ones and asserting the rest "unavailable" in prose — which lets a
    // task with live tooling (an MCP compile/browser/db call) degrade to a
    // fail-closed/exploratory outcome with no evidence it ever tried. Require a
    // recorded invocation of every declared tool (a captured failure counts as a
    // genuine attempt) before an accepted result is allowed to stand. Reads only
    // required_tools and commands_run — no tool-, domain-, or PRD-specific
    // knowledge, so it holds for every task, tool, and workflow engine.
    if result.status == WorkflowV2Status::Accepted {
        let unexercised = unexercised_required_tools(&request.input, result);
        if !unexercised.is_empty() {
            return Err(
                WorkflowV2AgentError::ImplementationAcceptedWithRequiredToolUnexercised(
                    unexercised,
                ),
            );
        }
    }
    match result.status {
        WorkflowV2Status::Accepted
            if result.files_changed.is_empty()
                && !has_project_artifact_evidence(result, &request.project_artifacts) =>
        {
            Err(WorkflowV2AgentError::ImplementationAcceptedWithoutChanges)
        }
        WorkflowV2Status::Noop if request_declares_required_tools(&request.input) => {
            // A task that declares required tools (e.g. project MCP tools)
            // cannot be satisfied by inspecting stale artifacts and declaring
            // a no-op: those tools must actually be exercised this run. Force
            // fresh work (accepted with real evidence) or an honest block.
            Err(WorkflowV2AgentError::ImplementationNoopWithDeclaredRequiredTools)
        }
        WorkflowV2Status::Noop if !has_typed_noop_proof(result) => {
            Err(WorkflowV2AgentError::ImplementationNoopWithoutTaskCoverage)
        }
        WorkflowV2Status::Noop
            if has_project_artifact_requirement(&request.input, &request.project_artifacts)
                && !has_project_artifact_evidence(result, &request.project_artifacts) =>
        {
            Err(WorkflowV2AgentError::ImplementationNoopMissingProjectArtifactEvidence)
        }
        _ => Ok(()),
    }
}

fn normalize_read_only_test_inspection(
    request: &WorkflowV2AgentRequest,
    result: &mut WorkflowV2Result,
) {
    if request.is_write_capable() || has_successful_test_command(result) {
        return;
    }
    for evidence in &mut result.evidence {
        if evidence.kind == WorkflowV2EvidenceKind::Test {
            evidence.kind = WorkflowV2EvidenceKind::Inspection;
        }
    }
    for coverage in &mut result.task_coverage {
        for evidence in &mut coverage.evidence {
            if evidence.kind == WorkflowV2EvidenceKind::Test {
                evidence.kind = WorkflowV2EvidenceKind::Inspection;
            }
        }
    }
}

fn has_successful_test_command(result: &WorkflowV2Result) -> bool {
    result.commands_run.iter().any(|command| {
        command.kind == WorkflowV2CommandKind::Test
            && command.status == WorkflowV2CommandStatus::Succeeded
            && !command.command.trim().is_empty()
    })
}

fn validate_write_ownership(
    request: &WorkflowV2AgentRequest,
    result: &mut WorkflowV2Result,
) -> Result<(), WorkflowV2AgentError> {
    if result.files_changed.is_empty() {
        return Ok(());
    }
    let repository_root = request.repository_root.as_deref();
    let target_files =
        normalize_targets_for_repository(&request.call.id, &request.target_files, repository_root)
            .map_err(|err| {
                WorkflowV2AgentError::ImplementationChangedFilesOutsideOwnership(err.to_string())
            })?;
    for file in &mut result.files_changed {
        file.path = normalize_target_for_repository(&request.call.id, &file.path, repository_root)
            .map_err(|err| {
                WorkflowV2AgentError::ImplementationChangedFilesOutsideOwnership(err.to_string())
            })?;
    }
    let write_item = WorkflowV2WriteItem::new(
        request.call.id.clone(),
        request
            .call
            .write_mode
            .unwrap_or(WorkflowV2WriteMode::Serial),
        target_files,
    )
    .with_owned_scopes(request.target_ownership_scopes.clone());
    validate_changed_files(&write_item, result).map_err(|err| {
        WorkflowV2AgentError::ImplementationChangedFilesOutsideOwnership(err.to_string())
    })
}

fn has_typed_noop_proof(result: &WorkflowV2Result) -> bool {
    result.task_coverage.iter().any(|coverage| {
        matches!(
            coverage.status,
            WorkflowV2TaskCoverageStatus::Noop | WorkflowV2TaskCoverageStatus::Accepted
        ) && coverage
            .evidence
            .iter()
            .any(|evidence| !evidence.summary.trim().is_empty())
    })
}

/// True when the stage input carries a non-empty `required_tools` (any casing)
/// anywhere in its structure. The write branch input embeds the task's
/// declared required_tools (stamped from the authoritative task universe), so
/// this recognizes tasks whose completion demands exercising specific tools.
fn request_declares_required_tools(input: &serde_json::Value) -> bool {
    match input {
        serde_json::Value::Object(object) => object.iter().any(|(key, value)| {
            if matches!(key.as_str(), "required_tools" | "requiredTools") {
                value
                    .as_array()
                    .is_some_and(|tools| tools.iter().any(|tool| tool.is_string()))
            } else {
                request_declares_required_tools(value)
            }
        }),
        serde_json::Value::Array(values) => values.iter().any(request_declares_required_tools),
        _ => false,
    }
}

/// EVERY declared required tool with no matching invocation in this result's
/// recorded commands — empty when all were exercised (or none were declared).
/// Matching is by the raw tool name against each command string,
/// case-insensitively, regardless of command status — a captured failure is a
/// genuine attempt and satisfies the requirement. Reads only `required_tools`
/// and `commands_run`, with no knowledge of any specific tool, domain, or PRD,
/// so the same guard holds for every workflow engine.
///
/// Reports all of them, not just the first. Naming one at a time turns a single
/// contract violation into a chain of rejections that each cost an attempt:
/// observed on TDL-041's review remediation, where attempt 2 was rejected for
/// `chart_get_state`, attempt 3 called it and was rejected for `quote_get`, and
/// the task ran out of attempts at 3 having needed 4. The agent can only fix
/// what the rejection told it about, so the rejection has to tell it everything.
fn unexercised_required_tools(input: &serde_json::Value, result: &WorkflowV2Result) -> Vec<String> {
    let mut required: Vec<String> = Vec::new();
    collect_required_tool_names(input, &mut required);
    if required.is_empty() {
        return Vec::new();
    }
    let commands: Vec<String> = result
        .commands_run
        .iter()
        .map(|command| command.command.to_ascii_lowercase())
        .collect();
    required
        .into_iter()
        .filter(|tool| {
            !commands
                .iter()
                .any(|command| command.contains(tool.as_str()))
        })
        .collect()
}

/// Collect the raw (lowercased) names of every declared required tool anywhere
/// in the stage input, stripping any `mcp__server__` qualifier down to the bare
/// tool name so a command referencing either the qualified or the raw name
/// matches.
fn collect_required_tool_names(input: &serde_json::Value, output: &mut Vec<String>) {
    match input {
        serde_json::Value::Object(object) => {
            for (key, value) in object {
                if matches!(key.as_str(), "required_tools" | "requiredTools") {
                    if let Some(items) = value.as_array() {
                        for name in items.iter().filter_map(serde_json::Value::as_str) {
                            let raw = raw_tool_name(name).to_ascii_lowercase();
                            if !raw.is_empty() && !output.contains(&raw) {
                                output.push(raw);
                            }
                        }
                    }
                } else {
                    collect_required_tool_names(value, output);
                }
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                collect_required_tool_names(value, output);
            }
        }
        _ => {}
    }
}

/// Strip an `mcp__server__` qualifier to the bare tool name; other names are
/// returned trimmed and unchanged.
fn raw_tool_name(name: &str) -> &str {
    name.strip_prefix("mcp__")
        .and_then(|suffix| suffix.split_once("__"))
        .map(|(_, raw)| raw)
        .unwrap_or(name)
        .trim()
}

