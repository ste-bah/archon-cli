use archon_observability::{AgentActivityKind, AgentActivityStatus};
use archon_tools::execution_deadline::ExecutionDeadline;
use archon_tools::repeat_tool_guard::{ChainKey, REPEAT_TOOL_CHAINS};
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
            observe_tool_attempt(ctx, tool.name(), &input, true);
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
        observe_tool_attempt(ctx, tool.name(), &input, true);
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
    let result = execute_within_budget(tool, input, ctx).await;
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
    observe_tool_attempt(ctx, tool.name(), &outcome_input, false);
    result
}

/// Extend this agent's repeat-tool chain by one attempt (#200 Phase 2).
///
/// Called at all three exits of [`execute_tool_attempt`] and deliberately
/// *not* folded into [`record_outcome`], which returns early when no outcome
/// callback is installed. A guard hanging off that callback would be present,
/// compiled, and silently inert in every context that has none — the shape
/// where a subsystem looks healthy and does nothing. This has no precondition
/// beyond the policy on the context, so turning it off is a config change and
/// never an accident of wiring.
///
/// `refused` marks an attempt that never reached the tool. It still extends
/// the run: a call permissions keep rejecting is the loop most worth breaking,
/// and it is the one whose result provably cannot change.
///
/// Records only. Nothing here can block, rewrite, or delay the call — the
/// advisory it may queue is delivered by the tool loop after the round's
/// results, as a separate user turn.
fn observe_tool_attempt(
    ctx: &ToolContext,
    tool_name: &str,
    input: &serde_json::Value,
    refused: bool,
) {
    REPEAT_TOOL_CHAINS.observe(
        &ChainKey::of(ctx),
        &ctx.repeat_tool,
        tool_name,
        input,
        refused,
    );
}

/// Run one tool call under the budget the tool declared for itself, if any.
///
/// The unbudgeted path is deliberately the bare `tool.execute(..).await` the
/// caller used before: a tool that returns `None` from [`Tool::timeout`] is not
/// wrapped, not polled through an extra layer, and behaves exactly as it did.
/// Of the 67 registered tools only a handful — the network- and IPC-bound ones
/// — declare a budget, and `Bash` is not among them: it enforces its own
/// deadline inside `bash_process.rs`, where it can kill the child it spawned,
/// and wrapping it here would give one command two competing clocks.
///
/// Expiry is reported as an ordinary error `ToolResult` rather than an unwind
/// or an abandoned turn. That matters because the alternative — letting a stall
/// propagate as anything other than a tool result — costs the model the one
/// thing it can act on. It gets told the call ran out of time and decides for
/// itself whether to retry, narrow the request, or give up. The caller still
/// emits the normal result activity with the elapsed time, so a timeout is
/// visible in telemetry as a failed call of known duration and not as a gap.
async fn execute_within_budget(
    tool: &dyn Tool,
    input: serde_json::Value,
    ctx: &ToolContext,
) -> ToolResult {
    let Some(budget) = tool.timeout() else {
        return tool.execute(input, ctx).await;
    };

    let deadline = ExecutionDeadline::new(budget);
    match deadline.wait(tool.execute(input, ctx)).await {
        Some(result) => result,
        None => ToolResult::error(format!(
            "{} timed out after {}ms",
            tool.name(),
            budget.as_millis()
        )),
    }
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

#[cfg(test)]
#[path = "tool_run_admission_timeout_tests.rs"]
mod timeout_tests;

#[cfg(test)]
#[path = "tool_run_admission_repeat_tool_tests.rs"]
mod repeat_tool_tests;
