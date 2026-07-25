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
                    "agent output must be one JSON WorkflowV2Result object: {err}"
                ))
            })?;
        let mut result: WorkflowV2Result = serde_json::from_value(value).map_err(|err| {
            WorkflowV2AgentError::MalformedOutput(format!(
                "agent output must be one JSON WorkflowV2Result object: {err}"
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
    match result.status {
        WorkflowV2Status::Accepted
            if result.files_changed.is_empty()
                && !has_project_artifact_evidence(result, &request.project_artifacts) =>
        {
            Err(WorkflowV2AgentError::ImplementationAcceptedWithoutChanges)
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

fn reject_forbidden_text(text: &str) -> Result<(), WorkflowV2AgentError> {
    let lower = text.to_ascii_lowercase();
    if lower.contains("restored context")
        || lower.contains("context restored")
        || lower.contains("previous-session summary")
        || lower.contains("previous session summary")
    {
        return Err(WorkflowV2AgentError::RestoredContextSummary);
    }
    if (lower.contains("should i ")
        || lower.contains("do you want me")
        || lower.contains("would you like me")
        || lower.contains("can i proceed"))
        && text.contains('?')
    {
        return Err(WorkflowV2AgentError::ConfirmationQuestion);
    }
    Ok(())
}

fn reject_forbidden_result_text(result: &WorkflowV2Result) -> Result<(), WorkflowV2AgentError> {
    reject_forbidden_text(&result.summary)?;
    for evidence in &result.evidence {
        reject_forbidden_text(&evidence.summary)?;
    }
    for coverage in &result.task_coverage {
        reject_forbidden_text(&coverage.summary)?;
        for evidence in &coverage.evidence {
            reject_forbidden_text(&evidence.summary)?;
        }
    }
    Ok(())
}

fn plan_only_text(result: &WorkflowV2Result) -> bool {
    let mut fields = vec![result.summary.as_str()];
    fields.extend(
        result
            .evidence
            .iter()
            .map(|evidence| evidence.summary.as_str()),
    );
    fields.iter().any(|field| {
        let lower = field.to_ascii_lowercase();
        lower.contains("i will ")
            || lower.contains("we will ")
            || lower.contains("would implement")
            || lower.contains("next steps")
            || lower.contains("proposed changes")
            || lower.contains("implementation plan")
    })
}

pub(super) fn write_mode_label(write_mode: Option<WorkflowV2WriteMode>) -> &'static str {
    match write_mode {
        Some(WorkflowV2WriteMode::Serial) => "serial",
        Some(WorkflowV2WriteMode::Coordinated) => "coordinated",
        Some(WorkflowV2WriteMode::Worktree) => "worktree",
        None => "read_only",
    }
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for ch in value.chars().take(max_chars) {
        out.push(ch);
    }
    out
}

pub(super) const READ_ONLY_RULES: &str = concat!(
    "- This is read-only work: do not claim file edits and leave files_changed empty.\n",
    "- For project artifact checks, use project_artifact_paths absolute_path values when present; otherwise resolve .archon/... paths under project_artifact_root, not repository_root."
);

pub(super) const IMPLEMENTATION_RULES: &str = concat!(
    "- This is implementation-capable work.\n",
    "- If edits are required and made, status must be accepted and files_changed must list each changed path.\n",
    "- The top-level status is your branch verdict; nested artifact/evidence content may describe fail-closed examples such as validation reports with status=failed.\n",
    "- commands_run.kind must be one of inspect, test, build, format, review, or other; use other for implementation notes.\n",
    "- Accepted/noop results must include concrete evidence: files/artifacts, commands_run, task_coverage, and residual_gaps when relevant.\n",
    "- Repository source edits must stay under repository_root and declared target_files; workflow/project artifacts must be written under project_artifact_root when provided and listed in artifacts.\n",
    "- If a write branch is genuinely already complete with no patch, return top-level \"status\":\"noop\", \"idempotent_noop\":true, commands_run evidence, and accepted/noop task_coverage evidence.\n",
    "- If no edits are required because the work is already complete, status must be noop and task_coverage must include typed evidence; declared project artifacts also require existing artifact evidence.\n",
    "- Status accepted with no files_changed is invalid unless concrete project artifact evidence was written under project_artifact_root."
);

pub(super) const FINAL_OUTPUT_RULE: &str = r#"## Final Output Rule
Your final message must be exactly one JSON WorkflowV2Result object, even for a no-op. Example: {"status":"noop","idempotent_noop":true,"summary":"already satisfied","evidence":[{"kind":"inspection","summary":"verified existing implementation"}],"commands_run":[{"kind":"inspect","command":"exact check","status":"succeeded","exit_code":0,"output_summary":"passed"}],"files_changed":[],"task_coverage":[{"task_id":"canonical task id","status":"noop","summary":"already satisfied","evidence":[{"kind":"implementation","summary":"concrete proof"}]}],"residual_gaps":[]}. Never return prose such as Status: noop."#;

pub(super) const RESULT_SCHEMA: &str = r#"{
  "status": "accepted | noop | failed | blocked | needs_review | cancelled",
  "idempotent_noop": "optional boolean; true only for a top-level noop with concrete evidence and no patch",
  "summary": "concise factual summary",
  "evidence": [{"kind": "inspection | implementation | test | review | remediation | blocker | artifact | other", "summary": "specific evidence", "source": "optional path or command"}],
  "artifacts": [{"id": "stable-id", "path": "artifact/path", "description": "optional"}],
  "commands_run": [{"kind": "inspect | test | build | format | review | other", "command": "exact command", "status": "succeeded | failed | skipped", "exit_code": 0, "output_summary": "short output"}],
  "files_read": [{"path": "path", "purpose": "optional"}],
  "files_changed": [{"path": "path", "purpose": "optional"}],
  "task_coverage": [{"task_id": "canonical id", "status": "accepted | noop | partial | missing | blocked | unknown", "summary": "coverage summary", "evidence": [{"kind": "implementation", "summary": "evidence"}]}],
  "residual_gaps": [{"id": "gap-id", "description": "remaining gap", "severity": "optional"}],
  "data": {"items": "optional typed payload for downstream fanout/reduce"}
}"#;

#[cfg(test)]
#[path = "agent_adapter_project_artifact_completion_tests.rs"]
mod project_artifact_completion_tests;
#[cfg(test)]
#[path = "agent_adapter_tests.rs"]
mod tests;
