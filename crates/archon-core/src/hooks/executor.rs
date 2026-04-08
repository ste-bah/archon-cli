use std::cell::Cell;
use std::path::Path;
use std::process::Stdio;

use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::Mutex as TokioMutex;

use super::context::HookContext;
use super::function::FunctionRegistry;
use super::types::{HookConfig, HookEvent, HookOutcome, HookResult};

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

/// Lazy-initialized FunctionRegistry for function hooks.
static FUNCTION_REGISTRY: std::sync::LazyLock<FunctionRegistry> =
    std::sync::LazyLock::new(FunctionRegistry::new);

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
    Timeout,
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spawn(s) => write!(f, "spawn error: {s}"),
            Self::Io(s) => write!(f, "I/O error: {s}"),
            Self::Timeout => write!(f, "timed out"),
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
/// - Any other code or error → `HookResult { outcome: NonBlockingError, .. }` (logged)
///
/// If `config.async == Some(true)`, the command is spawned in the background
/// and a Success result is returned immediately without waiting.
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
        let client = reqwest::Client::new();
        return super::http::execute_http_hook(config, input, &client).await;
    }

    // Prompt hooks: run command, capture stdout as plain text (NOT JSON-parsed)
    if matches!(config.hook_type, super::types::HookCommandType::Prompt) {
        return execute_prompt_hook(config, input, cwd, session_id, event_name).await;
    }

    // Async: fire-and-forget, return Allow immediately.
    if config.r#async == Some(true) {
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
        Err(e) => {
            tracing::warn!(
                hook = %config.command,
                error = %e,
                "hook execution failed (non-blocking, returning Allow)"
            );
            HookResult::allow()
        }
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
        Err(e) => {
            tracing::warn!(
                hook = %config.command,
                error = %e,
                "agent hook execution failed (fail-open, returning Allow)"
            );
            HookResult::allow()
        }
    }
    // AgentGuard dropped here -> set_in_hook_agent(false)
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
// Shell command runner
// ---------------------------------------------------------------------------

async fn run_command(
    command: &str,
    payload_bytes: &[u8],
    cwd: &Path,
    session_id: &str,
    event_name: &str,
    timeout_secs: u32,
) -> Result<CommandOutput, RunError> {
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(cwd)
        .env("ARCHON_SESSION_ID", session_id)
        .env("ARCHON_CWD", cwd.to_string_lossy().as_ref())
        .env("ARCHON_HOOK_EVENT", event_name)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| RunError::Spawn(format!("{command}: {e}")))?;

    // Write payload to stdin then drop so the child gets EOF.
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(payload_bytes).await;
    }

    let timeout = std::time::Duration::from_secs(u64::from(timeout_secs));

    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(output)) => {
            let exit_code = output.status.code().unwrap_or(-1);
            let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            Ok(CommandOutput {
                exit_code,
                stdout,
                stderr,
            })
        }
        Ok(Err(e)) => Err(RunError::Io(e.to_string())),
        Err(_) => Err(RunError::Timeout),
    }
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
            2 => {
                let reason = if output.stderr.trim().is_empty() {
                    format!("prompt hook '{}' blocked (exit 2)", config.command)
                } else {
                    output.stderr.trim().to_owned()
                };
                HookResult::block(reason)
            }
            _ => {
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
        },
        Err(e) => {
            tracing::warn!(
                hook = %config.command,
                error = %e,
                "prompt hook execution failed (non-blocking, returning Allow)"
            );
            HookResult::allow()
        }
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
            Ok(b) => b,
            Err(_) => return,
        };
        let _ = run_command(
            &command,
            &payload_bytes,
            &cwd,
            &session_id,
            &event_name,
            timeout_secs,
        )
        .await;
    });
}

// ---------------------------------------------------------------------------
// Function hook executor — in-process named function dispatch
// ---------------------------------------------------------------------------

fn execute_function_hook(
    config: &HookConfig,
    input: &serde_json::Value,
    cwd: &Path,
    session_id: &str,
    event_name: &str,
) -> HookResult {
    // Parse event name back to HookEvent for context building.
    let hook_event: HookEvent =
        serde_json::from_value(serde_json::Value::String(event_name.to_string()))
            .unwrap_or(HookEvent::PreToolUse);

    let tool_name = input
        .get("tool_name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let mut builder = HookContext::builder(hook_event)
        .session_id(session_id.to_string())
        .cwd(cwd.to_string_lossy().to_string());

    if let Some(name) = tool_name {
        builder = builder.tool_name(name);
    }

    if let Some(tool_input) = input.get("tool_input") {
        builder = builder.tool_input(tool_input.clone());
    }

    let ctx = builder.build();
    FUNCTION_REGISTRY.execute(&config.command, &ctx)
}
