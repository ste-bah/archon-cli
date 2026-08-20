use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::time::Duration;

use process_wrap::tokio::ChildWrapper;
use tokio::task::JoinHandle;

use crate::cargo_target_env::CargoTargetDirLock;
use crate::execution_deadline::{ExecutionDeadline, abort_pipe_tasks, join_pipe_tasks};

use super::bash_containment::{contained_bash_command, terminate_completed_process_tree};
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
    // `CARGO_INCREMENTAL=0` used to be set here, ahead of the resource defaults.
    // Since `ensure_env_default` is first-wins, that made `[tools.cargo]
    // incremental` unreachable, so the setting now lives entirely in
    // `apply_workflow_resource_defaults` — which still applies it to every
    // command, exactly as this line did.
    crate::workflow_resource_env::apply_workflow_resource_defaults(
        &mut env_vars,
        raw_command,
        &tool.cargo_limits,
    );
    // Inside a workflow run the session id IS the run id, so a task that
    // declares the run's identifier as a required env key gets it without the
    // operator exporting a value that is stale the moment the next run starts.
    crate::workflow_run_env::apply_workflow_run_identity(
        &mut env_vars,
        &ctx.session_id,
        &tool.run_id_env_aliases,
    );
    // An agent allowed to build inside its own worktree builds into a scratch
    // directory beside it, so prune can remove the build output along with the
    // checkout. Without this the build lands in the worktree's own `target/`
    // and disappears with it — which sounds fine until the worktree is *kept*,
    // and the gigabytes are kept with it invisibly (#184 M3).
    if let Some(subagent_id) = ctx.subagent_id.as_deref()
        && tool.isolation_tier == crate::isolation::IsolationTier::WorktreeWithBuilds
    {
        let owner_id = crate::worktree_ownership::subagent_owner_key(subagent_id);
        let scratch = crate::worktree_manager::WorktreeManager::scratch_target_dir(&owner_id);
        env_vars.push((
            "CARGO_TARGET_DIR".to_string(),
            scratch.to_string_lossy().into_owned(),
        ));
    }

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
    raw_command: &str,
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
    let content = redact_provider_env_output(prepared.provider_env.as_ref(), result.content);
    let result = match result.exit_code {
        Some(exit_code) => ToolResult::from_authoritative_bash_execution(
            content,
            ctx.session_id.clone(),
            ctx.tool_run_tool_use_id.clone().unwrap_or_default(),
            ctx.tool_run_attempt,
            raw_command.to_string(),
            exit_code,
        ),
        None => ToolResult::from_parts(content, result.is_error),
    };
    Some(limit_tool_result(tool.max_output_bytes, result))
}

pub(super) fn limit_tool_result(max_output_bytes: usize, mut result: ToolResult) -> ToolResult {
    result.content = bounded_text(result.content, max_output_bytes);
    result
}

pub(super) fn bash_result_from_pipes(
    max_output_bytes: usize,
    ctx: &ToolContext,
    command: &str,
    stdout: CapturedOutput,
    stderr: CapturedOutput,
    exit_code: i32,
) -> ToolResult {
    let output = bounded_command_output(stdout, stderr, exit_code, max_output_bytes);
    ToolResult::from_authoritative_bash_execution(
        output,
        ctx.session_id.clone(),
        ctx.tool_run_tool_use_id.clone().unwrap_or_default(),
        ctx.tool_run_attempt,
        command.to_string(),
        exit_code,
    )
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
        ctx,
        raw_command,
        prepared.provider_env.as_ref(),
        result,
    );
    drop(prepared.cargo_lock);
    result
}

/// The exit code to report for a finished process.
///
/// `ExitStatus::code()` is `None` on Unix when the process was killed by a
/// signal, and collapsing that to `-1` loses which signal — an OOM kill and a
/// `SIGSEGV` came back identical, and `BashOutcome` is otherwise a clean
/// `Done`/`Timeout`/`Cancelled` split with no other place two outcomes report
/// as one (#193).
///
/// `128 + signal` is the convention every shell uses for `$?`, so the number
/// matches what the same command would have shown at a prompt.
fn reported_exit_code(status: &std::process::ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return 128 + signal;
        }
    }
    -1
}

fn redact_and_limit_result(
    max_output_bytes: usize,
    ctx: &ToolContext,
    raw_command: &str,
    provider_env: Option<&ProviderEnvResolution>,
    mut result: ToolResult,
) -> ToolResult {
    let content = redact_provider_env_output(provider_env, result.content);
    result.content = bounded_text(content, max_output_bytes);
    if let Some(exit_code) = result
        .authoritative_bash_execution()
        .map(|execution| execution.exit_code())
    {
        return ToolResult::from_authoritative_bash_execution(
            result.content,
            ctx.session_id.clone(),
            ctx.tool_run_tool_use_id.clone().unwrap_or_default(),
            ctx.tool_run_attempt,
            raw_command.to_string(),
            exit_code,
        );
    }
    result
}

pub(super) fn spawn_bash_child(
    ctx: &ToolContext,
    prepared: &PreparedBashCommand,
) -> std::io::Result<Box<dyn ChildWrapper>> {
    let mut command = contained_bash_command(&prepared.command);
    command
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
    finish_bash_outcome(tool, ctx, raw_command, prepared, &mut state, outcome).await
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
    pub(super) async fn fail(&mut self, reason: &str, message: String) -> ToolResult {
        let cleanup_error = terminate_child(self.child, self.process_group, reason).await;
        abort_pipe_tasks(self.stdout_task, self.stderr_task);
        if reason == "parent cancellation" {
            tracing::info!("bash: command cancelled by parent CancellationToken");
        }
        let message = match cleanup_error {
            Some(error) => format!("{message}; process cleanup failed: {error}"),
            None => message,
        };
        ToolResult::error(message)
    }
}

pub(super) async fn finish_bash_outcome(
    tool: &BashTool,
    ctx: &ToolContext,
    raw_command: &str,
    prepared: &PreparedBashCommand,
    state: &mut BashChildState<'_>,
    outcome: BashOutcome,
) -> ToolResult {
    let failure = match outcome {
        BashOutcome::Done(status) => {
            return completed_bash_result(tool, ctx, raw_command, prepared, state, status).await;
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
    ctx: &ToolContext,
    raw_command: &str,
    prepared: &PreparedBashCommand,
    state: &mut BashChildState<'_>,
    status: std::io::Result<std::process::ExitStatus>,
) -> ToolResult {
    let status = match status {
        Ok(status) => status,
        Err(error) => {
            return state
                .fail(
                    "process wait failure",
                    format!("Failed to wait for bash process: {error}"),
                )
                .await;
        }
    };
    if let Some(error) = terminate_completed_process_tree(state.process_group) {
        abort_pipe_tasks(state.stdout_task, state.stderr_task);
        return ToolResult::error(format!(
            "[BASH_PROCESS_TREE_INCOMPLETE] Bash exited but its process group could not be terminated: {error}"
        ));
    }
    let (stdout, stderr) =
        match join_pipe_tasks(state.deadline, state.stdout_task, state.stderr_task).await {
            Some(pipes) => pipes,
            None => {
                return state
                    .fail(
                        "pipe drain timeout",
                        format!("Command timed out after {}ms", prepared.timeout_ms),
                    )
                    .await;
            }
        };
    if let Some(error) = stdout.read_error.as_ref().or(stderr.read_error.as_ref()) {
        return state
            .fail(
                "pipe read failure",
                format!("Failed to capture bash output: {error}"),
            )
            .await;
    }
    let exit_code = reported_exit_code(&status);
    bash_result_from_pipes(
        tool.max_output_bytes,
        ctx,
        raw_command,
        stdout,
        stderr,
        exit_code,
    )
}

pub(super) async fn terminate_child(
    child: &mut Box<dyn ChildWrapper>,
    process_group: Option<u32>,
    reason: &str,
) -> Option<String> {
    #[cfg(not(unix))]
    let _ = process_group;
    #[cfg(unix)]
    let kill_error = terminate_completed_process_tree(process_group);
    #[cfg(not(unix))]
    let kill_error = child.start_kill().err().map(|error| error.to_string());
    let wait_error = match tokio::time::timeout(Duration::from_secs(2), child.wait()).await {
        Ok(Ok(_)) => None,
        Ok(Err(error)) => Some(error.to_string()),
        Err(_) => Some("process reap exceeded 2 second cleanup deadline".to_string()),
    };
    let cleanup_error = kill_error.or(wait_error);
    if let Some(error) = &cleanup_error {
        tracing::warn!(reason, error, "bash: process-tree cleanup failed");
    } else {
        tracing::info!(reason, "bash: terminated process tree");
    }
    cleanup_error
}
