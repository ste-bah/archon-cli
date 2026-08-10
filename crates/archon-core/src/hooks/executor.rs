use std::cell::Cell;
use std::path::Path;

#[path = "executor_process.rs"]
mod executor_process;
use executor_process::run_command;
use tokio::sync::Mutex as TokioMutex;

use super::types::{HookConfig, HookOutcome, HookResult};

#[path = "executor_function.rs"]
mod executor_function;
use executor_function::execute_function_hook;

#[cfg(test)]
#[path = "executor_tests.rs"]
mod executor_tests;

// ---------------------------------------------------------------------------
// Agent hook recursion guard (thread-local) and serialization mutex
// ---------------------------------------------------------------------------

thread_local! {
    static IN_HOOK_AGENT: Cell<bool> = const { Cell::new(false) };
}

/// Check if currently inside an agent hook (recursion guard).
pub fn is_in_hook_agent() -> bool {
    IN_HOOK_AGENT.with(|flag| flag.get())
}

/// Set the agent hook recursion guard flag.
pub fn set_in_hook_agent(value: bool) {
    IN_HOOK_AGENT.with(|flag| flag.set(value));
}

/// Lazy-initialized Mutex for agent hook serialization (max concurrency: 1).
static AGENT_HOOK_MUTEX: std::sync::LazyLock<TokioMutex<()>> =
    std::sync::LazyLock::new(|| TokioMutex::new(()));

/// RAII guard that resets IN_HOOK_AGENT to false on drop.
struct AgentGuard;

impl Drop for AgentGuard {
    fn drop(&mut self) {
        set_in_hook_agent(false);
    }
}

// ---------------------------------------------------------------------------
// Internal result from running a shell command
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct CommandOutput {
    exit_code: i32,
    stdout: String,
    stderr: String,
}

// ---------------------------------------------------------------------------
// Internal error (never propagated — hooks always return HookResult)
// ---------------------------------------------------------------------------

#[derive(Debug)]
enum RunError {
    Spawn(String),
    Io(String),
    Timeout(&'static str),
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spawn(s) => write!(f, "spawn error: {s}"),
            Self::Io(s) => write!(f, "I/O error: {s}"),
            Self::Timeout(phase) => write!(f, "timed out during {phase}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Public hook executor
// ---------------------------------------------------------------------------

/// Execute a single `HookConfig` command.
///
/// **Exit code semantics:**
/// - `0` → `HookResult { outcome: Success, .. }` (may include stdout JSON fields)
/// - `2` → `HookResult { outcome: Blocking, reason: stderr, .. }`
/// - Any other code → `HookResult { outcome: NonBlockingError, .. }` (logged)
/// - Spawn, I/O, or timeout failure → configured/event-default failure policy
///
/// If `config.async == Some(true)` and the hook's failure policy allows it,
/// the command is spawned in the background and a Success result is returned
/// immediately without waiting.
pub(crate) async fn execute_hook(
    config: &HookConfig,
    input: &serde_json::Value,
    cwd: &Path,
    session_id: &str,
    event_name: &str,
) -> HookResult {
    // Function hooks: in-process execution, no shell spawn needed
    if matches!(config.hook_type, super::types::HookCommandType::Function) {
        return execute_function_hook(config, input, cwd, session_id, event_name);
    }

    // Agent hooks: serialized with recursion guard
    if matches!(config.hook_type, super::types::HookCommandType::Agent) {
        return execute_agent_hook(config, input, cwd, session_id, event_name).await;
    }

    // Http hooks use a different execution path
    if matches!(config.hook_type, super::types::HookCommandType::Http) {
        return super::http::execute_http_hook_for_event(
            config,
            input,
            super::http::shared_client(),
            event_name,
        )
        .await;
    }

    // Prompt hooks: run command, capture stdout as plain text (NOT JSON-parsed)
    if matches!(config.hook_type, super::types::HookCommandType::Prompt) {
        return execute_prompt_hook(config, input, cwd, session_id, event_name).await;
    }

    // Fire-and-forget is only compatible with fail-open behavior. Hooks whose
    // failures block must finish before the guarded operation can proceed.
    if config.r#async == Some(true)
        && config.failure_policy(event_name) == super::types::HookFailurePolicy::Allow
    {
        spawn_background(
            config.command.clone(),
            input.clone(),
            cwd.to_path_buf(),
            session_id.to_owned(),
            event_name.to_owned(),
            config.timeout.unwrap_or(60),
        );
        return HookResult::allow();
    }

    let payload_bytes = match serde_json::to_vec(input) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, "hook: failed to serialize input payload");
            return HookResult::allow();
        }
    };

    let timeout_secs = config.timeout.unwrap_or(60);

    match run_command(
        &config.command,
        &payload_bytes,
        cwd,
        session_id,
        event_name,
        timeout_secs,
    )
    .await
    {
        Ok(output) => interpret_exit_code(&config.command, output),
        Err(e) => hook_failure_result(config, event_name, &e),
    }
}

// ---------------------------------------------------------------------------
// Agent hook executor — runs command under mutex with recursion guard
// ---------------------------------------------------------------------------

async fn execute_agent_hook(
    config: &HookConfig,
    input: &serde_json::Value,
    cwd: &Path,
    session_id: &str,
    event_name: &str,
) -> HookResult {
    // Acquire mutex so only one agent hook runs at a time.
    let _lock = AGENT_HOOK_MUTEX.lock().await;

    // Set recursion guard and create RAII cleanup.
    set_in_hook_agent(true);
    let _guard = AgentGuard;

    let payload_bytes = match serde_json::to_vec(input) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, "agent hook: failed to serialize input");
            return HookResult::allow();
        }
    };

    let timeout_secs = config.timeout.unwrap_or(60);

    match run_command(
        &config.command,
        &payload_bytes,
        cwd,
        session_id,
        event_name,
        timeout_secs,
    )
    .await
    {
        Ok(output) => interpret_exit_code(&config.command, output),
        Err(e) => hook_failure_result(config, event_name, &e),
    }
    // AgentGuard dropped here -> set_in_hook_agent(false)
}

fn hook_failure_result(config: &HookConfig, event_name: &str, error: &RunError) -> HookResult {
    tracing::warn!(
        hook = %config.command,
        error = %error,
        policy = ?config.failure_policy(event_name),
        "hook execution failed"
    );

    config.failure_result(event_name, &error.to_string())
}

// ---------------------------------------------------------------------------
// Exit code interpretation + stdout JSON parsing (REQ-HOOK-002)
// ---------------------------------------------------------------------------

fn interpret_exit_code(command: &str, output: CommandOutput) -> HookResult {
    // Step 1: Try to parse stdout as JSON HookResult
    let stdout_parsed = if !output.stdout.trim().is_empty() {
        match serde_json::from_str::<HookResult>(&output.stdout) {
            Ok(parsed) => Some(parsed),
            Err(e) => {
                tracing::warn!(
                    hook = %command,
                    error = %e,
                    "hook stdout is not valid HookResult JSON, falling back to exit-code behavior"
                );
                None
            }
        }
    } else {
        None
    };

    // Step 2: Base result from exit code
    let mut result = match output.exit_code {
        0 => HookResult::allow(),
        2 => {
            let reason = if output.stderr.trim().is_empty() {
                format!("hook '{command}' blocked tool execution (exit 2)")
            } else {
                output.stderr.trim().to_owned()
            };
            HookResult::block(reason)
        }
        code => {
            tracing::warn!(
                hook = %command,
                exit_code = code,
                stderr = %output.stderr.trim(),
                "hook exited with non-zero code (non-blocking failure)"
            );
            HookResult {
                outcome: HookOutcome::NonBlockingError,
                reason: Some(format!("exit code {code}")),
                ..Default::default()
            }
        }
    };

    // Step 3: If stdout parsed successfully, overlay fields onto base result.
    // Safety: exit=2 keeps Blocking outcome regardless of stdout.
    if let Some(parsed) = stdout_parsed {
        if output.exit_code != 2 {
            result.outcome = parsed.outcome;
        }
        if parsed.reason.is_some() {
            result.reason = parsed.reason;
        }
        if parsed.system_message.is_some() {
            result.system_message = parsed.system_message;
        }
        if parsed.updated_input.is_some() {
            result.updated_input = parsed.updated_input;
        }
        if parsed.permission_behavior.is_some() {
            result.permission_behavior = parsed.permission_behavior;
        }
        if parsed.permission_decision_reason.is_some() {
            result.permission_decision_reason = parsed.permission_decision_reason;
        }
        if parsed.updated_mcp_tool_output.is_some() {
            result.updated_mcp_tool_output = parsed.updated_mcp_tool_output;
        }
        if parsed.additional_context.is_some() {
            result.additional_context = parsed.additional_context;
        }
        if parsed.prevent_continuation.is_some() {
            result.prevent_continuation = parsed.prevent_continuation;
        }
        if parsed.stop_reason.is_some() {
            result.stop_reason = parsed.stop_reason;
        }
        if parsed.retry.is_some() {
            result.retry = parsed.retry;
        }
        if parsed.status_message.is_some() {
            result.status_message = parsed.status_message;
        }
        if parsed.source_authority.is_some() {
            result.source_authority = parsed.source_authority;
        }
        if !parsed.updated_permissions.is_empty() {
            result.updated_permissions = parsed.updated_permissions;
        }
        if !parsed.watch_paths.is_empty() {
            result.watch_paths = parsed.watch_paths;
        }
        if parsed.elicitation_action.is_some() {
            result.elicitation_action = parsed.elicitation_action;
        }
        if parsed.elicitation_content.is_some() {
            result.elicitation_content = parsed.elicitation_content;
        }
    }

    result
}

// ---------------------------------------------------------------------------
// Prompt hook executor — stdout as plain text, not JSON-parsed
// ---------------------------------------------------------------------------

async fn execute_prompt_hook(
    config: &HookConfig,
    input: &serde_json::Value,
    cwd: &Path,
    session_id: &str,
    event_name: &str,
) -> HookResult {
    let payload_bytes = match serde_json::to_vec(input) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, "prompt hook: failed to serialize input payload");
            return HookResult::allow();
        }
    };

    let timeout_secs = config.timeout.unwrap_or(60);

    match run_command(
        &config.command,
        &payload_bytes,
        cwd,
        session_id,
        event_name,
        timeout_secs,
    )
    .await
    {
        Ok(output) => match output.exit_code {
            0 => {
                let trimmed = output.stdout.trim();
                let additional_context = if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                };
                HookResult {
                    additional_context,
                    ..HookResult::allow()
                }
            }
            2 => {
                let reason = if output.stderr.trim().is_empty() {
                    format!("prompt hook '{}' blocked (exit 2)", config.command)
                } else {
                    output.stderr.trim().to_owned()
                };
                HookResult::block(reason)
            }
            code => {
                tracing::warn!(
                    hook = %config.command,
                    exit_code = code,
                    stderr = %output.stderr.trim(),
                    "prompt hook exited with non-zero code"
                );
                HookResult {
                    outcome: HookOutcome::NonBlockingError,
                    reason: Some(format!("exit code {code}")),
                    ..Default::default()
                }
            }
        },
        Err(e) => hook_failure_result(config, event_name, &e),
    }
}

// ---------------------------------------------------------------------------
// Background (async: true) execution
// ---------------------------------------------------------------------------

fn spawn_background(
    command: String,
    input: serde_json::Value,
    cwd: std::path::PathBuf,
    session_id: String,
    event_name: String,
    timeout_secs: u32,
) {
    tokio::spawn(async move {
        let payload_bytes = match serde_json::to_vec(&input) {
            Ok(bytes) => bytes,
            Err(error) => {
                tracing::warn!(
                    hook = %command,
                    event = %event_name,
                    error = %error,
                    "failed to serialize background hook payload"
                );
                return;
            }
        };
        if let Err(error) = run_command(
            &command,
            &payload_bytes,
            &cwd,
            &session_id,
            &event_name,
            timeout_secs,
        )
        .await
        {
            tracing::warn!(
                hook = %command,
                event = %event_name,
                error = %error,
                "background hook execution failed"
            );
        }
    });
}
