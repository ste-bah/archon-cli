use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::{
    WorkflowV2CommandKind, WorkflowV2CommandStatus, WorkflowV2EvidenceKind, WorkflowV2HostCall,
    WorkflowV2Result, WorkflowV2Status, WorkflowV2TaskCoverageStatus, WorkflowV2WriteItem,
    WorkflowV2WriteMode, validate_changed_files,
};

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
    #[serde(default)]
    pub target_files: Vec<String>,
}

impl WorkflowV2AgentRequest {
    pub fn is_write_capable(&self) -> bool {
        self.call.write_mode.is_some()
            || self.role.eq_ignore_ascii_case("coder")
            || self.role.eq_ignore_ascii_case("implementation")
    }
}

#[derive(Debug, Clone, Default)]
pub struct WorkflowV2AgentAdapter;

impl WorkflowV2AgentAdapter {
    pub fn new() -> Self {
        Self
    }

    pub fn build_prompt(&self, request: &WorkflowV2AgentRequest) -> String {
        let input = serde_json::to_string_pretty(&request.input)
            .unwrap_or_else(|_| request.input.to_string());
        let target_files = if request.target_files.is_empty() {
            "[]".to_string()
        } else {
            serde_json::to_string(&request.target_files).unwrap_or_else(|_| "[]".to_string())
        };
        let constraints = if request.constraints.is_empty() {
            "[]".to_string()
        } else {
            serde_json::to_string_pretty(&request.constraints).unwrap_or_else(|_| "[]".to_string())
        };
        let write_rules = if request.is_write_capable() {
            IMPLEMENTATION_RULES
        } else {
            READ_ONLY_RULES
        };

        format!(
            "## Archon Workflow V2 Agent Call\n\
             call_id: {call_id}\n\
             role: {role}\n\
             write_mode: {write_mode}\n\
             repository_root: {repository_root}\n\
             target_files: {target_files}\n\n\
             ## Task\n{task}\n\n\
             ## Constraints\n```json\n{constraints}\n```\n\n\
             ## Input\n```json\n{input}\n```\n\n\
             ## Execution Rules\n\
             - Execute the requested work now; do not ask a confirmation question.\n\
             - Return exactly one JSON object and no markdown fence, prose prefix, or prose suffix.\n\
             - Do not return restored-context summaries or previous-session summaries.\n\
             - Do not stop at a plan or proposed next steps for executable work.\n\
             {write_rules}\n\n\
             ## Required JSON Result Envelope\n\
             {RESULT_SCHEMA}\n",
            call_id = request.call.id,
            role = request.role,
            write_mode = write_mode_label(request.call.write_mode),
            repository_root = request.repository_root.as_deref().unwrap_or("<none>"),
            target_files = target_files,
            task = request.task,
            constraints = constraints,
            input = input,
        )
    }

    pub fn build_repair_prompt(
        &self,
        request: &WorkflowV2AgentRequest,
        invalid_output: &str,
        error: &WorkflowV2AgentError,
    ) -> String {
        format!(
            "The previous workflow V2 agent response for call '{}' was invalid.\n\n\
             Error: {error}\n\n\
             Return exactly one JSON object matching the required result envelope. \
             Do not include markdown fences, restored-context summaries, confirmation questions, \
             provider names, model names, or plan-only text.\n\n\
             Task:\n{}\n\n\
             Required JSON Result Envelope:\n{RESULT_SCHEMA}\n\n\
             Previous invalid output excerpt:\n{}\n",
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
        let mut result: WorkflowV2Result = serde_json::from_str(output).map_err(|err| {
            WorkflowV2AgentError::MalformedOutput(format!(
                "agent output must be one JSON WorkflowV2Result object: {err}"
            ))
        })?;
        normalize_read_only_test_inspection(request, &mut result);
        result.validate().map_err(|err| {
            WorkflowV2AgentError::InvalidResult(format!("agent result failed validation: {err}"))
        })?;
        validate_request_specific_result(request, &result)?;
        Ok(result)
    }

    pub async fn run_with_repair<C>(
        &self,
        client: &C,
        request: &WorkflowV2AgentRequest,
    ) -> Result<WorkflowV2Result, WorkflowV2AgentError>
    where
        C: WorkflowV2AgentClient + Sync,
    {
        let prompt = self.build_prompt(request);
        let first = client.run_agent_request(request, prompt).await?;
        match self.parse_agent_output(request, &first) {
            Ok(result) => Ok(result),
            Err(first_error) => {
                let repair_prompt = self.build_repair_prompt(request, &first, &first_error);
                let repaired = client.run_agent_request(request, repair_prompt).await?;
                self.parse_agent_output(request, &repaired)
                    .map_err(|repair_error| WorkflowV2AgentError::RepairExhausted {
                        first_error: Box::new(first_error),
                        repair_error: Box::new(repair_error),
                    })
            }
        }
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

#[derive(Debug, Error, PartialEq, Eq)]
pub enum WorkflowV2AgentError {
    #[error("{0}")]
    MalformedOutput(String),
    #[error("{0}")]
    InvalidResult(String),
    #[error("agent output contains restored-context summary text")]
    RestoredContextSummary,
    #[error("agent output contains a confirmation question instead of executing")]
    ConfirmationQuestion,
    #[error(
        "implementation agent returned a plan-only result instead of edits or typed no-op proof"
    )]
    PlanOnlyImplementation,
    #[error(
        "implementation agent returned accepted status without changed files; use noop with task coverage evidence when no edits are required"
    )]
    ImplementationAcceptedWithoutChanges,
    #[error("implementation noop requires typed task_coverage evidence")]
    ImplementationNoopWithoutTaskCoverage,
    #[error("implementation agent changed files outside declared target_files: {0}")]
    ImplementationChangedFilesOutsideOwnership(String),
    #[error("read-only agent result must not claim changed files")]
    ReadOnlyChangedFiles,
    #[error("agent transport failed: {0}")]
    Transport(String),
    #[error("schema repair failed after one retry: first={first_error}; repair={repair_error}")]
    RepairExhausted {
        first_error: Box<WorkflowV2AgentError>,
        repair_error: Box<WorkflowV2AgentError>,
    },
}

fn validate_request_specific_result(
    request: &WorkflowV2AgentRequest,
    result: &WorkflowV2Result,
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
    validate_write_ownership(request, result)?;
    match result.status {
        WorkflowV2Status::Accepted if result.files_changed.is_empty() => {
            Err(WorkflowV2AgentError::ImplementationAcceptedWithoutChanges)
        }
        WorkflowV2Status::Noop if !has_typed_noop_proof(result) => {
            Err(WorkflowV2AgentError::ImplementationNoopWithoutTaskCoverage)
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
    result: &WorkflowV2Result,
) -> Result<(), WorkflowV2AgentError> {
    if result.files_changed.is_empty() {
        return Ok(());
    }
    let write_item = WorkflowV2WriteItem::new(
        request.call.id.clone(),
        request
            .call
            .write_mode
            .unwrap_or(WorkflowV2WriteMode::Serial),
        request.target_files.clone(),
    );
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

fn write_mode_label(write_mode: Option<WorkflowV2WriteMode>) -> &'static str {
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

const READ_ONLY_RULES: &str =
    "- This is read-only work: do not claim file edits and leave files_changed empty.";

const IMPLEMENTATION_RULES: &str = concat!(
    "- This is implementation-capable work.\n",
    "- If edits are required and made, status must be accepted and files_changed must list each changed path.\n",
    "- If no edits are required because the work is already complete, status must be noop and task_coverage must include typed evidence.\n",
    "- Status accepted with no files_changed is invalid for implementation work."
);

const RESULT_SCHEMA: &str = r#"{
  "status": "accepted | noop | failed | blocked | needs_review | cancelled",
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
