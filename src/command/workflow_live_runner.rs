use std::path::PathBuf;
use std::sync::Arc;

use archon_workflow::{
    ProviderTier, SharedWorkflowUiSink, StageKind, StageRunOutput, StageRunRequest,
    WorkflowActivityStatus, WorkflowAgentCall, WorkflowAgentSpec, WorkflowAgentToolAccess,
    WorkflowLlmClient, WorkflowStageRunner, WriteBoundaryProbe,
};

use super::workflow_agent_select::select_workflow_agent_key;
use super::workflow_live_items::{item_output_needs_schema_repair, repair_item_output};
use super::workflow_live_prompt::workflow_prompt;
use super::workflow_live_retry;
use super::workflow_live_runner_activity::required_activity;

pub(crate) struct PipelineWorkflowRunner {
    pub(crate) llm: Arc<dyn WorkflowLlmClient>,
    pub(crate) ui_sink: SharedWorkflowUiSink,
    pub(crate) agent_names: Vec<String>,
    pub(crate) workspace_boundary_supported: bool,
}

impl WriteBoundaryProbe for PipelineWorkflowRunner {
    fn supports_workspace_boundary(&self) -> bool {
        self.workspace_boundary_supported
    }
}

#[async_trait::async_trait]
impl WorkflowStageRunner for PipelineWorkflowRunner {
    fn max_concurrency(&self) -> Option<usize> {
        let from_executor = archon_tools::subagent_executor::get_subagent_executor()
            .and_then(|exec| exec.max_concurrency());
        Some(
            from_executor.unwrap_or(archon_core::subagent::SubagentManager::DEFAULT_MAX_CONCURRENT),
        )
    }

    async fn run_stage(
        &self,
        request: StageRunRequest,
    ) -> archon_workflow::WorkflowResult<StageRunOutput> {
        let model_alias = tier_model_alias(request.provider_tier).to_string();
        let resolved_model = self.llm.resolve_model_alias(&model_alias);
        let provider_id = self
            .llm
            .provider_id()
            .unwrap_or_else(|| "active-provider".to_string());
        let agent = workflow_agent(&request, &model_alias, &self.agent_names);
        let agent_name = agent.key.clone();
        required_activity(
            &self.ui_sink,
            &request,
            &agent_name,
            &provider_id,
            &resolved_model,
            WorkflowActivityStatus::Running,
            "stage running",
        )
        .await?;
        let agent_request = WorkflowAgentCall {
            session_id: workflow_agent_session_id(&request),
            task: request.task.clone(),
            cwd: request_target_repository_root(&request),
            ordinal: workflow_agent_ordinal(&request),
            attempt: request.attempt as usize,
            agent,
            messages: vec![serde_json::json!({
                "role": "user",
                "content": workflow_prompt(&request),
            })],
            system: vec![serde_json::json!({
                "type": "text",
                "text": workflow_stage_system_context(&request),
            })],
            tools: Vec::new(),
            allowed_tools: allowed_tools(&request),
            timeout_secs: None,
            disable_auto_background: false,
            provider_env: None,
        };
        let response = match workflow_live_retry::run_agent_with_transient_retry(
            &self.llm,
            agent_request.clone(),
            |attempt| {
                let ui_sink = self.ui_sink.clone();
                let request = request.clone();
                let agent_name = agent_name.clone();
                let provider_id = provider_id.clone();
                let resolved_model = resolved_model.clone();
                async move {
                    required_activity(
                        &ui_sink,
                        &request,
                        &agent_name,
                        &provider_id,
                        &resolved_model,
                        WorkflowActivityStatus::Running,
                        &format!("stage retrying after transient provider error ({attempt}/3)"),
                    )
                    .await
                }
            },
        )
        .await
        {
            Ok(response) => response,
            Err(err) => {
                required_activity(
                    &self.ui_sink,
                    &request,
                    &agent_name,
                    &provider_id,
                    &resolved_model,
                    WorkflowActivityStatus::Failed,
                    "stage failed",
                )
                .await?;
                return Err(err);
            }
        };
        let response = if item_output_needs_schema_repair(&request, &response.content) {
            required_activity(
                &self.ui_sink,
                &request,
                &agent_name,
                &provider_id,
                &resolved_model,
                WorkflowActivityStatus::Running,
                "stage repairing invalid item output",
            )
            .await?;
            match repair_item_output(
                &self.llm,
                &request,
                &agent_request,
                response,
                |attempt| {
                    let ui_sink = self.ui_sink.clone();
                    let request = request.clone();
                    let agent_name = agent_name.clone();
                    let provider_id = provider_id.clone();
                    let resolved_model = resolved_model.clone();
                    async move {
                        required_activity(
                            &ui_sink,
                            &request,
                            &agent_name,
                            &provider_id,
                            &resolved_model,
                            WorkflowActivityStatus::Running,
                            &format!("stage item-output repair retrying after transient provider error ({attempt}/3)"),
                        )
                        .await
                    }
                },
            )
            .await
            {
                Ok(response) => response,
                Err(err) => {
                    required_activity(
                        &self.ui_sink,
                        &request,
                        &agent_name,
                        &provider_id,
                        &resolved_model,
                        WorkflowActivityStatus::Failed,
                        "stage failed item-output repair",
                    )
                    .await?;
                    return Err(err);
                }
            }
        } else {
            response
        };
        required_activity(
            &self.ui_sink,
            &request,
            &agent_name,
            &provider_id,
            &resolved_model,
            WorkflowActivityStatus::Complete,
            "stage complete",
        )
        .await?;
        let mut output = StageRunOutput::markdown(response.content);
        output.provider_id = Some(provider_id);
        output.resolved_model = Some(resolved_model);
        output.tokens_in = response.tokens_in;
        output.tokens_out = response.tokens_out;
        output.tool_uses = response
            .tool_uses
            .into_iter()
            .map(|entry| {
                serde_json::json!({
                    "tool_name": entry.tool_name,
                    "input": entry.input,
                    "output": entry.output,
                })
            })
            .collect();
        Ok(output)
    }
}

pub(crate) fn workflow_agent_ordinal(request: &StageRunRequest) -> usize {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in request.stage_id.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    let suffix = (hash % 999_983) as usize + 1;
    (request.attempt.max(1) as usize)
        .saturating_mul(1_000_000)
        .saturating_add(suffix)
}

pub(crate) fn workflow_agent_session_id(request: &StageRunRequest) -> String {
    format!(
        "{}-stage-{}-attempt-{}",
        request.run_id,
        sanitize_agent_session_component(&request.stage_id),
        request.attempt.max(1)
    )
}

fn sanitize_agent_session_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len().min(96));
    for ch in value.chars() {
        if out.len() >= 96 {
            break;
        }
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "stage".to_string()
    } else {
        trimmed.to_string()
    }
}

pub(crate) fn workflow_stage_system_context(request: &StageRunRequest) -> String {
    format!(
        "You are an Archon dynamic workflow stage agent. This is a fresh workflow stage invocation for run '{}', stage '{}', attempt {}. Ignore any restored conversational context, prior subagent memory, or earlier session summary that conflicts with this invocation. Follow only the current Workflow Task, Evidence Contract, Stage Input, and output schema. Do not ask what to do next, do not stop at a confirmation question, and do not return restored-context summaries. Return only useful public output for the stage artifact. Do not include private reasoning, hidden chain-of-thought, credentials, or provider internals.",
        request.run_id,
        request.stage_id,
        request.attempt.max(1)
    )
}

pub(crate) fn request_target_repository_root(request: &StageRunRequest) -> Option<PathBuf> {
    request
        .input
        .get("target_repository_root")
        .and_then(|value| value.as_str())
        .filter(|path| !path.trim().is_empty())
        .map(PathBuf::from)
}

pub(crate) fn workflow_agent(
    request: &StageRunRequest,
    model: &str,
    agent_names: &[String],
) -> WorkflowAgentSpec {
    let key = select_workflow_agent_key(request, agent_names);
    WorkflowAgentSpec {
        display_name: key.replace('-', " "),
        key,
        model: model.to_string(),
        phase: 0,
        critical: matches!(request.stage_kind, StageKind::QualityGate),
        parallelizable: matches!(request.stage_kind, StageKind::Fanout),
        quality_threshold: 0.5,
        tool_access: if matches!(request.stage_kind, StageKind::Implementation)
            || command_execution_stage(request)
        {
            WorkflowAgentToolAccess::Full
        } else {
            WorkflowAgentToolAccess::ReadOnly
        },
    }
}

pub(crate) fn allowed_tools(request: &StageRunRequest) -> Vec<String> {
    let tools = match request.stage_kind {
        StageKind::Implementation => implementation_tools(request),
        _ if command_execution_stage(request) => {
            vec!["Read", "Grep", "Glob", "Bash", "DocSearch", "DocGet"]
        }
        StageKind::Tool => vec!["Read", "Grep", "Glob", "DocSearch", "DocGet"],
        _ => vec![
            "Read",
            "Grep",
            "Glob",
            "WebSearch",
            "WebFetch",
            "DocSearch",
            "DocGet",
        ],
    };
    let mut tools = tools.into_iter().map(str::to_string).collect::<Vec<_>>();
    tools.extend(super::workflow_live_mcp::allowed_mcp_tools(request));
    tools
}

fn implementation_tools(_request: &StageRunRequest) -> Vec<&'static str> {
    // Every implementation branch gets Bash. Without it a coder can Write/Edit
    // but cannot run `cargo check`/`cargo test`/`cargo build` to verify its own
    // edits — so on live runs it hesitated ("performed only safe read-only
    // inspection; confirm before I make edits") and never implemented. Bash was
    // previously withheld under write-coordination unless the task text happened
    // to mention running tests, which silently crippled most coder branches.
    // Concurrent worktree builds serialize on cargo's own target-dir lock, and
    // any file changed via Bash without being declared is still caught by
    // validate_changed_files_for_repository at merge time.
    vec![
        "Read",
        "Grep",
        "Glob",
        "Write",
        "Edit",
        "ApplyPatch",
        "LargeEditBegin",
        "LargeEditInsertAfter",
        "LargeEditReplaceSection",
        "LargeEditDeleteSection",
        "LargeEditCommit",
        "LargeEditAbort",
        "Bash",
    ]
}

pub(crate) fn command_execution_stage(request: &StageRunRequest) -> bool {
    if stage_extra_requests_bash(request) {
        return true;
    }
    // Typed V2 calls decide from declared fields: focused-verification waves
    // run commands; every other read-only call gets no shell. Prose sniffing
    // is reserved for requests without a typed call.
    if request.input.get("v2_call").is_some() {
        let id = request.stage_id.to_ascii_lowercase().replace('-', "_");
        if id.starts_with("verification_wave_") || id.starts_with("review_verification_wave_") {
            return true;
        }
        if generated_v2_read_only_call(request) {
            return false;
        }
    }
    if command_execution_stage_id(&request.stage_id) {
        return true;
    }
    let haystack = format!(
        "{}\n{}\n{}\n{}",
        request.stage_id,
        request.task,
        request
            .input
            .get("stage_task")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default(),
        request.input
    )
    .to_ascii_lowercase();
    command_execution_text(&haystack)
}

fn generated_v2_read_only_call(request: &StageRunRequest) -> bool {
    let Some(v2_call) = request.input.get("v2_call") else {
        return false;
    };
    let write_mode = v2_call.get("write_mode");
    if write_mode.is_some_and(|value| !value.is_null()) {
        return false;
    }
    let method = v2_call
        .get("method")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    matches!(
        method,
        "agent" | "fanout" | "parallel" | "reduce" | "finalReport" | "qualityGate" | "humanGate"
    )
}

fn command_execution_stage_id(stage_id: &str) -> bool {
    let id = stage_id.to_ascii_lowercase().replace('-', "_");
    id.starts_with("verification_wave_")
        || id.starts_with("review_verification_wave_")
        || id.ends_with("_tests")
        || id.contains("_post_tests")
        || id.contains("_focused_tests")
        || id.contains("_verification")
}

fn command_execution_text(haystack: &str) -> bool {
    [
        "focused_test",
        "focused-test",
        "focused test",
        "focused tests",
        "post-remediation tests",
        "post remediation tests",
        "cargo test",
        "test command",
        "test execution",
        "test evidence",
        "run tests",
        "run focused",
        "tests and checks",
        "verification",
        "verify",
        "quality gate",
        "cargo check",
        "cargo build",
        "cargo fmt",
        "rustfmt",
        "clippy",
        "lint",
    ]
    .iter()
    .any(|needle| haystack.contains(needle))
}

fn stage_extra_requests_bash(request: &StageRunRequest) -> bool {
    let Some(extra) = request.input.get("stage_extra") else {
        return false;
    };
    ["allowed_tools", "tools", "required_tools"]
        .iter()
        .filter_map(|key| extra.get(*key))
        .flat_map(text_values)
        .any(|tool| tool.eq_ignore_ascii_case("bash") || tool.eq_ignore_ascii_case("shell"))
}

fn text_values(value: &serde_json::Value) -> Vec<&str> {
    match value {
        serde_json::Value::String(value) => vec![value.as_str()],
        serde_json::Value::Array(values) => values
            .iter()
            .filter_map(serde_json::Value::as_str)
            .collect(),
        _ => Vec::new(),
    }
}

pub(crate) fn tier_model_alias(tier: ProviderTier) -> &'static str {
    match tier {
        ProviderTier::Cheap | ProviderTier::Local => "haiku",
        ProviderTier::Critic | ProviderTier::Reducer => "opus",
        ProviderTier::Planner
        | ProviderTier::Researcher
        | ProviderTier::Coder
        | ProviderTier::Vision => "sonnet",
    }
}
