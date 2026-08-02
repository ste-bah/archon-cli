use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::time::Duration;

use process_wrap::tokio::ChildWrapper;
use tokio::process::Command;
use tokio::task::JoinHandle;

use crate::cargo_target_env::CargoTargetDirLock;
use crate::execution_deadline::{ExecutionDeadline, abort_pipe_tasks, join_pipe_tasks};

use super::bash_env::ensure_env_default;
use super::bash_output::{
    CapturedOutput, bounded_command_output, bounded_text, shared_output_budget,
    spawn_counted_pipe_capture, spawn_wrapped_child,
};
use super::*;

pub(super) async fn prepare_command(
    tool: &BashTool,
    raw_command: &str,
    timeout_ms: u64,
    ctx: &ToolContext,
) -> Result<PreparedBashCommand, ToolResult> {
    let mut env_vars = sanitized_env();
    ensure_env_default(&mut env_vars, "CARGO_INCREMENTAL", "0");
    crate::workflow_resource_env::apply_workflow_resource_defaults(&mut env_vars, raw_command);
    let provider_env = provider_env_overlay(tool.provider_env.as_ref()).await;
    if let Some(provider_env) = &provider_env {
        provider_env.apply_to_env(&mut env_vars);
    }
    let cargo_lock = cargo_target_lock(&mut env_vars, raw_command, ctx).await?;
    let command = guarded_bash_command(raw_command, cargo_lock.as_ref());
    Ok(PreparedBashCommand {
        command,
        env_vars,
        provider_env,
        cargo_lock,
        timeout_ms,
    })
}

pub(super) struct PreparedBashCommand {
    command: String,
    env_vars: Vec<(String, String)>,
    provider_env: Option<ProviderEnvResolution>,
    cargo_lock: Option<CargoTargetDirLock>,
    timeout_ms: u64,
}

pub(super) fn command_from_input(input: &serde_json::Value) -> Result<&str, ToolResult> {
    input
        .get("command")
        .and_then(|value| value.as_str())
        .ok_or_else(|| ToolResult::error("command is required and must be a string"))
}

pub(super) async fn cargo_target_lock(
    env_vars: &mut Vec<(String, String)>,
    command: &str,
    ctx: &ToolContext,
) -> Result<Option<CargoTargetDirLock>, ToolResult> {
    crate::cargo_target_env::apply_cargo_target_dir_guard(
        env_vars,
        command,
        &ctx.working_dir,
        &ctx.session_id,
        ctx.cancel_parent.clone(),
    )
    .await
    .map_err(ToolResult::error)
}

pub(super) fn guarded_bash_command(
    command: &str,
    cargo_lock: Option<&CargoTargetDirLock>,
) -> String {
    let command =
        crate::cargo_target_env::enforce_host_cargo_target_dir(command, cargo_lock.is_some());
    let prelude = crate::cargo_target_env::cargo_cache_repair_prelude(cargo_lock);
    let command = if prelude.is_empty() {
        command
    } else {
        format!("{prelude}\n{command}")
    };
    command_with_compat_prelude(&command)
}

pub(super) async fn execute_in_sandbox(
    tool: &BashTool,
    ctx: &ToolContext,
    prepared: &PreparedBashCommand,
) -> Option<ToolResult> {
    let sandbox = ctx.sandbox.as_ref()?;
    let result = sandbox
        .execute_bash(archon_permissions::sandbox::SandboxCommandRequest {
            command: prepared.command.clone(),
            working_dir: ctx.working_dir.clone(),
            timeout_ms: prepared.timeout_ms,
            max_output_bytes: tool.max_output_bytes,
            env: prepared.env_vars.clone(),
        })
        .await?;
    Some(limit_tool_result(
        tool.max_output_bytes,
        ToolResult {
            content: redact_provider_env_output(prepared.provider_env.as_ref(), result.content),
            is_error: result.is_error,
        },
    ))
}

pub(super) fn limit_tool_result(max_output_bytes: usize, result: ToolResult) -> ToolResult {
    ToolResult {
        content: bounded_text(result.content, max_output_bytes),
        is_error: result.is_error,
    }
}

pub(super) fn bash_result_from_pipes(
    max_output_bytes: usize,
    stdout: CapturedOutput,
    stderr: CapturedOutput,
    exit_code: i32,
) -> ToolResult {
    let output = bounded_command_output(stdout, stderr, exit_code, max_output_bytes);
    ToolResult {
        content: output,
        is_error: exit_code != 0,
    }
}

pub(super) async fn run_prepared_bash_command(
    tool: &BashTool,
    ctx: &ToolContext,
    raw_command: &str,
    prepared: PreparedBashCommand,
) -> ToolResult {
    let deadline = ExecutionDeadline::new(Duration::from_millis(prepared.timeout_ms));
    let mut child = match spawn_bash_child(ctx, &prepared) {
        Ok(child) => child,
        Err(error) => {
            return limit_tool_result(
                tool.max_output_bytes,
                ToolResult::error(format!("Failed to spawn bash: {error}")),
            );
        }
    };
    let result = await_bash_child(tool, ctx, raw_command, &prepared, &deadline, &mut child).await;
    let result = redact_and_limit_result(
        tool.max_output_bytes,
        prepared.provider_env.as_ref(),
        result,
    );
    drop(prepared.cargo_lock);
    result
}

fn redact_and_limit_result(
    max_output_bytes: usize,
    provider_env: Option<&ProviderEnvResolution>,
    result: ToolResult,
) -> ToolResult {
    limit_tool_result(
        max_output_bytes,
        ToolResult {
            content: redact_provider_env_output(provider_env, result.content),
            is_error: result.is_error,
        },
    )
}

pub(super) fn spawn_bash_child(
    ctx: &ToolContext,
    prepared: &PreparedBashCommand,
) -> std::io::Result<Box<dyn ChildWrapper>> {
    let mut command = Command::new(BASH_PROGRAM.as_path());
    command
        .arg("-c")
        .arg(&prepared.command)
        .current_dir(&ctx.working_dir)
        .env_clear()
        .envs(prepared.env_vars.clone())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());
    spawn_wrapped_child(command)
}

pub(super) async fn await_bash_child(
    tool: &BashTool,
    ctx: &ToolContext,
    raw_command: &str,
    prepared: &PreparedBashCommand,
    deadline: &ExecutionDeadline,
    child: &mut Box<dyn ChildWrapper>,
) -> ToolResult {
    let process_group = child.id();
    let (mut stdout_task, mut stderr_task, heartbeat) =
        start_bash_observers(tool, ctx, raw_command, prepared.timeout_ms, child);
    let outcome = wait_for_bash_outcome(ctx, deadline, child).await;
    crate::bash_observability::stop_bash_heartbeat(heartbeat);
    let mut state = BashChildState {
        child,
        process_group,
        deadline,
        stdout_task: &mut stdout_task,
        stderr_task: &mut stderr_task,
    };
    finish_bash_outcome(tool, prepared, &mut state, outcome).await
}

pub(super) fn start_bash_observers(
    tool: &BashTool,
    ctx: &ToolContext,
    raw_command: &str,
    timeout_ms: u64,
    child: &mut Box<dyn ChildWrapper>,
) -> (
    JoinHandle<CapturedOutput>,
    JoinHandle<CapturedOutput>,
    Option<JoinHandle<()>>,
) {
    let remaining = shared_output_budget(tool.max_output_bytes);
    let stdout_bytes = Arc::new(AtomicUsize::new(0));
    let stderr_bytes = Arc::new(AtomicUsize::new(0));
    let heartbeat = crate::bash_observability::start_bash_heartbeat(
        ctx,
        child.id(),
        timeout_ms,
        raw_command,
        Arc::clone(&stdout_bytes),
        Arc::clone(&stderr_bytes),
    );
    let stdout =
        spawn_counted_pipe_capture(child.stdout().take(), Arc::clone(&remaining), stdout_bytes);
    let stderr = spawn_counted_pipe_capture(child.stderr().take(), remaining, stderr_bytes);
    (stdout, stderr, heartbeat)
}

pub(super) enum BashOutcome {
    Done(std::io::Result<std::process::ExitStatus>),
    Timeout,
    Cancelled,
}

pub(super) async fn wait_for_bash_outcome(
    ctx: &ToolContext,
    deadline: &ExecutionDeadline,
    child: &mut Box<dyn ChildWrapper>,
) -> BashOutcome {
    let cancel_token = ctx.cancel_parent.clone().unwrap_or_default();
    tokio::select! {
        biased;
        _ = cancel_token.cancelled() => BashOutcome::Cancelled,
        result = deadline.wait(child.wait()) => match result {
            Some(status) => BashOutcome::Done(status),
            None => BashOutcome::Timeout,
        }
    }
}

pub(super) struct BashChildState<'a> {
    child: &'a mut Box<dyn ChildWrapper>,
    process_group: Option<u32>,
    deadline: &'a ExecutionDeadline,
    stdout_task: &'a mut JoinHandle<CapturedOutput>,
    stderr_task: &'a mut JoinHandle<CapturedOutput>,
}

impl BashChildState<'_> {
    async fn fail(&mut self, reason: &str, message: String) -> ToolResult {
        terminate_child(self.child, self.process_group, reason, self.deadline).await;
        abort_pipe_tasks(self.stdout_task, self.stderr_task);
        if reason == "parent cancellation" {
            tracing::info!("bash: command cancelled by parent CancellationToken");
        }
        ToolResult::error(message)
    }
}

pub(super) async fn finish_bash_outcome(
    tool: &BashTool,
    prepared: &PreparedBashCommand,
    state: &mut BashChildState<'_>,
    outcome: BashOutcome,
) -> ToolResult {
    let failure = match outcome {
        BashOutcome::Done(status) => {
            return completed_bash_result(tool, prepared, state, status).await;
        }
        BashOutcome::Timeout => (
            "timeout",
            format!("Command timed out after {}ms", prepared.timeout_ms),
        ),
        BashOutcome::Cancelled => (
            "parent cancellation",
            "Command cancelled by user".to_string(),
        ),
    };
    state.fail(failure.0, failure.1).await
}

pub(super) async fn completed_bash_result(
    tool: &BashTool,
    prepared: &PreparedBashCommand,
    state: &mut BashChildState<'_>,
    status: std::io::Result<std::process::ExitStatus>,
) -> ToolResult {
    let Some((stdout, stderr)) =
        join_pipe_tasks(state.deadline, state.stdout_task, state.stderr_task).await
    else {
        return state
            .fail(
                "pipe drain timeout",
                format!("Command timed out after {}ms", prepared.timeout_ms),
            )
            .await;
    };
    let exit_code = status
        .as_ref()
        .ok()
        .and_then(|status| status.code())
        .unwrap_or(-1);
    bash_result_from_pipes(tool.max_output_bytes, stdout, stderr, exit_code)
}

pub(super) async fn terminate_child(
    child: &mut Box<dyn ChildWrapper>,
    process_group: Option<u32>,
    reason: &str,
    deadline: &ExecutionDeadline,
) {
    #[cfg(unix)]
    if let Some(pid) = process_group {
        // SAFETY: the wrapped command is the process-group leader created above.
        unsafe { libc::kill(-(pid as libc::pid_t), libc::SIGKILL) };
    }
    #[cfg(not(unix))]
    let _ = child.start_kill();
    let _ = deadline.wait(child.wait()).await;
    tracing::info!(reason, "bash: terminated process tree");
}
