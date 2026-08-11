use std::sync::Arc;

use archon_tools::board::DelegatedOutcome;
use archon_workflow::{
    ProviderTier, SharedWorkflowUiSink, StageKind, StageRunOutput, StageRunRequest,
    WorkflowActivityStatus, WorkflowAgentCall, WorkflowAgentSpec, WorkflowAgentToolAccess,
    WorkflowLlmClient, WorkflowStageRunner, WriteBoundaryProbe,
};

/// Child of this module, not a sibling: the board item's identity is derived
/// from the same session id and ordinal this file mints, and the two belong
/// where they can be read together.
#[path = "workflow_live_stage_board.rs"]
pub(crate) mod workflow_live_stage_board;

use archon_workflow::agent_select::select_workflow_agent_key;
use archon_workflow::llm_retry::run_agent_with_transient_retry;
use archon_workflow::stage_activity::{request_target_repository_root, required_activity};
use archon_workflow::stage_command_policy::command_execution_stage;
use archon_workflow::stage_item_output::{item_output_needs_schema_repair, repair_item_output};
use archon_workflow::stage_prompt::{workflow_prompt, workflow_stage_system_context};
use workflow_live_stage_board::{StageBoardItem, stage_board_outcome};

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
        // On the board before the provider is called, and claimed, because that
        // is what is true from this point on: an agent is holding this branch.
        // Raised here rather than after the call so a stage that dies in the
        // provider still leaves a record of having been dispatched.
        let session_id = workflow_agent_session_id(&request);
        let ordinal = workflow_agent_ordinal(&request);
        let prompt = workflow_prompt(&request);
        let mut board = StageBoardItem::raise(&request, &session_id, ordinal, &agent_name, &prompt);
        let agent_request = WorkflowAgentCall {
            session_id,
            task: request.task.clone(),
            cwd: request_target_repository_root(&request),
            ordinal,
            attempt: request.attempt as usize,
            agent,
            messages: vec![serde_json::json!({
                "role": "user",
                "content": prompt,
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
        let response =
            match run_agent_with_transient_retry(&self.llm, agent_request.clone(), |attempt| {
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
            })
            .await
            {
                Ok(response) => response,
                Err(err) => {
                    // Before the emit, not after: the emit is a `?`, and the
                    // classified verdict has to survive it unwinding.
                    board.finish(stage_board_outcome(&err));
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
                    board.finish(stage_board_outcome(&err));
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
        // `Completed` closes to `in_review`, not `resolved`: all that was
        // observed is a branch returning content, and nothing has checked that
        // the work it claims to have done exists.
        board.finish(DelegatedOutcome::Completed);
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
