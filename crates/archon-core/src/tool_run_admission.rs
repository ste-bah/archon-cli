use archon_observability::{AgentActivityKind, AgentActivityStatus};
use archon_tools::tool::{
    PermissionLevel, Tool, ToolContext, ToolResult, ToolRunAdmission, ToolRunAdmissionRequest,
    ToolRunAttemptOutcome,
};

use crate::agent::events::emit_tool_result_activity;

pub(crate) async fn execute_tool_attempt(
    tool: &dyn Tool,
    input: serde_json::Value,
    ctx: &ToolContext,
    sandbox_prechecked: bool,
) -> ToolResult {
    let permission_level = tool.permission_level(&input);
    // Admission still runs only for non-`Safe` tools with a callback installed
    // — that is a policy decision and it has not changed. What changed is that
    // the *outcome* callback below no longer inherits that filter.
    let admission_enabled =
        permission_level != PermissionLevel::Safe && ctx.tool_run_admission.is_some();
    if admission_enabled && let Some(admission) = &ctx.tool_run_admission {
        let request = admission_request(ctx, tool.name(), &input, permission_level);
        if let ToolRunAdmission::Blocked { reason } = admission(request) {
            let result = ToolResult::error(format!("ToolRun blocked: {reason}"));
            crate::dispatch::emit_tool_activity(
                ctx,
                tool.name(),
                AgentActivityKind::ToolFailed,
                AgentActivityStatus::Failed,
            );
            record_outcome(
                ctx,
                tool.name(),
                &input,
                permission_level,
                true,
                true,
                admission_enabled,
            );
            return result;
        }
    }

    if !sandbox_prechecked
        && let Some(backend) = &ctx.sandbox
        && let Err(reason) = backend.check(tool.name(), tool.capability(), &input)
    {
        crate::dispatch::emit_tool_activity(
            ctx,
            tool.name(),
            AgentActivityKind::ToolFailed,
            AgentActivityStatus::Failed,
        );
        record_outcome(
            ctx,
            tool.name(),
            &input,
            permission_level,
            false,
            true,
            admission_enabled,
        );
        return ToolResult::error(reason);
    }

    crate::dispatch::emit_tool_activity(
        ctx,
        tool.name(),
        AgentActivityKind::ToolStarted,
        AgentActivityStatus::Running,
    );
    let started_at = std::time::Instant::now();
    let outcome_input = input.clone();
    let result = tool.execute(input, ctx).await;
    emit_tool_result_activity(ctx, tool.name(), &result, started_at.elapsed());
    record_outcome(
        ctx,
        tool.name(),
        &outcome_input,
        permission_level,
        false,
        result.is_error,
        admission_enabled,
    );
    result
}

fn admission_request(
    ctx: &ToolContext,
    tool_name: &str,
    input: &serde_json::Value,
    permission_level: PermissionLevel,
) -> ToolRunAdmissionRequest {
    ToolRunAdmissionRequest {
        session_id: ctx.session_id.clone(),
        parent_action_id: ctx.tool_run_parent_action_id.clone().unwrap_or_default(),
        tool_use_id: ctx.tool_run_tool_use_id.clone().unwrap_or_default(),
        attempt: ctx.tool_run_attempt,
        tool_name: tool_name.to_string(),
        input: input.clone(),
        permission_level,
    }
}

/// Report a terminal outcome for a tool attempt.
///
/// **Fires for every attempt**, including `Safe` tools and attempts for which no
/// admission callback is installed. It used to fire only when admission ran,
/// which made it useless as an ambient signal: `Safe` covers the great majority
/// of tool calls, so a topology trace built on the old behaviour would have seen
/// almost nothing.
///
/// The cost of widening it is that the one existing consumer — the world-model
/// guardrail in `src/command/world_model/guard/00_tool_run.rs` — was written
/// against the narrow contract. It resolves each outcome to a persisted
/// admission decision and, finding none, warns and writes an
/// `unavailable` record. Left alone it would have done that for every `Safe`
/// tool call in the process, both spamming the log and polluting the guardrail
/// outcome store with rows describing attempts that were never guarded.
/// `admission_evaluated` carries the distinction the filter used to imply, and
/// that consumer now returns early on `false`.
fn record_outcome(
    ctx: &ToolContext,
    tool_name: &str,
    input: &serde_json::Value,
    permission_level: PermissionLevel,
    blocked: bool,
    is_error: bool,
    admission_evaluated: bool,
) {
    let Some(callback) = &ctx.tool_run_outcome else {
        return;
    };
    callback(ToolRunAttemptOutcome {
        session_id: ctx.session_id.clone(),
        parent_action_id: ctx.tool_run_parent_action_id.clone().unwrap_or_default(),
        tool_use_id: ctx.tool_run_tool_use_id.clone().unwrap_or_default(),
        attempt: ctx.tool_run_attempt,
        tool_name: tool_name.to_string(),
        input: input.clone(),
        permission_level,
        blocked,
        is_error,
        admission_evaluated,
    });
}
