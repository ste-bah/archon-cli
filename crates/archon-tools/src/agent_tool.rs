use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::background_agents::{
    AgentStatus, BACKGROUND_AGENTS, BackgroundAgentHandle, RegistryError, new_result_slot,
};
use crate::subagent_executor::{SubagentClassification, SubagentOutcome, get_subagent_executor};
use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

// ---------------------------------------------------------------------------
// Subagent request — returned as JSON for the caller (agent loop) to handle
// ---------------------------------------------------------------------------

/// A validated request to spawn a subagent.  The `AgentTool` does not actually
/// spawn anything — it validates parameters and produces this struct so the
/// outer agent loop can orchestrate the real subagent lifecycle.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubagentRequest {
    pub prompt: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    pub max_turns: u32,
    pub timeout_secs: u64,
    /// When set, loads a custom agent definition for this subagent.
    #[serde(default)]
    pub subagent_type: Option<String>,
    /// Per-call background override. When true, subagent runs as a background task.
    #[serde(default)]
    pub run_in_background: bool,
    /// Working directory override for the subagent.
    #[serde(default)]
    pub cwd: Option<String>,
    /// When set to "worktree", the subagent runs in an isolated git worktree.
    #[serde(default)]
    pub isolation: Option<String>,
}

impl SubagentRequest {
    /// Default maximum turns when the caller does not specify one.
    pub const DEFAULT_MAX_TURNS: u32 = 10;

    /// Default timeout in seconds.
    pub const DEFAULT_TIMEOUT_SECS: u64 = 300;
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum AgentToolError {
    #[error("missing required field: {0}")]
    MissingField(&'static str),

    #[error("invalid input: {0}")]
    InvalidInput(String),
}

// ---------------------------------------------------------------------------
// AgentTool — implements Tool
// ---------------------------------------------------------------------------

pub struct AgentTool {
    /// Dynamic description including available agents. Built at registration time.
    description: String,
}

impl AgentTool {
    /// Create an AgentTool with default description (no agent listing).
    pub fn new() -> Self {
        Self {
            description:
                "Spawn a subagent to handle a complex task autonomously. Returns a SubagentRequest \
                for the agent loop to execute. The subagent runs with its own conversation and \
                tool set."
                    .into(),
        }
    }

    /// Create an AgentTool with an injected agent listing.
    /// The listing is appended to the description so the LLM knows valid subagent_type values.
    pub fn with_agent_listing(agents: &[(String, String)]) -> Self {
        let mut desc =
            "Spawn a subagent to handle a complex task autonomously. Returns a SubagentRequest \
            for the agent loop to execute. The subagent runs with its own conversation and \
            tool set."
                .to_string();

        if !agents.is_empty() {
            desc.push_str("\n\nAvailable agents: ");
            let entries: Vec<String> = agents
                .iter()
                .map(|(name, summary)| {
                    if summary.is_empty() {
                        name.clone()
                    } else {
                        format!("{name} ({summary})")
                    }
                })
                .collect();
            desc.push_str(&entries.join(", "));
        }

        Self { description: desc }
    }
}

impl Default for AgentTool {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentTool {
    fn validate_and_build(
        &self,
        input: &serde_json::Value,
    ) -> Result<SubagentRequest, AgentToolError> {
        let prompt = input
            .get("prompt")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .ok_or(AgentToolError::MissingField("prompt"))?
            .to_string();

        let model = input
            .get("model")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let allowed_tools = match input.get("allowed_tools") {
            Some(serde_json::Value::Array(arr)) => arr
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect(),
            _ => Vec::new(),
        };

        let max_turns = input
            .get("max_turns")
            .and_then(|v| v.as_u64())
            .map(|n| {
                if n == 0 || n > 100 {
                    Err(AgentToolError::InvalidInput(
                        "max_turns must be between 1 and 100".into(),
                    ))
                } else {
                    Ok(n as u32)
                }
            })
            .transpose()?
            .unwrap_or(SubagentRequest::DEFAULT_MAX_TURNS);

        let timeout_secs = input
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(SubagentRequest::DEFAULT_TIMEOUT_SECS);

        let subagent_type = input
            .get("subagent_type")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string());

        let run_in_background = input
            .get("run_in_background")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let cwd = input
            .get("cwd")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string());

        let isolation = input
            .get("isolation")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string());

        Ok(SubagentRequest {
            prompt,
            model,
            allowed_tools,
            max_turns,
            timeout_secs,
            subagent_type,
            run_in_background,
            cwd,
            isolation,
        })
    }
}

#[async_trait::async_trait]
impl Tool for AgentTool {
    fn name(&self) -> &str {
        "Agent"
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["prompt"],
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "The task prompt for the subagent"
                },
                "model": {
                    "type": "string",
                    "description": "Model to use for the subagent (defaults to parent model)"
                },
                "allowed_tools": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "List of tool names the subagent is allowed to use"
                },
                "max_turns": {
                    "type": "integer",
                    "description": "Maximum conversation turns (default 10, max 100)"
                },
                "subagent_type": {
                    "type": "string",
                    "description": "Optional agent type name. When set, loads the agent's custom prompt and tool filters."
                },
                "run_in_background": {
                    "type": "boolean",
                    "description": "When true, runs the subagent as a background task."
                },
                "cwd": {
                    "type": "string",
                    "description": "Working directory override for the subagent."
                },
                "isolation": {
                    "type": "string",
                    "enum": ["worktree"],
                    "description": "Isolation mode. 'worktree' creates a temporary git worktree for the subagent."
                }
            }
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        // TASK-AGS-105: `AgentTool::execute` routes through the installed
        // `SubagentExecutor` via `run_subagent`. Two top-level branches:
        //
        //   - ExplicitBackground (run_in_background: true): spawn
        //     `run_subagent` into a detached task, register the handle
        //     in BACKGROUND_AGENTS, return `{agent_id, status:"spawned"}`
        //     synchronously. Preserves the TASK-AGS-104 background
        //     contract byte-for-byte.
        //   - Foreground (default): spawn `run_subagent`, await the
        //     outcome, map per the Section 2d matrix (Completed → real
        //     text; Failed → error; AutoBackgrounded → spawn marker with
        //     the exact pre-allocated id; Cancelled → error).
        //
        // See docs/task-ags-105-mapping.md Sections 2c + 2d for the
        // full contract.
        let request = match self.validate_and_build(&input) {
            Ok(req) => req,
            Err(e) => return ToolResult::error(e.to_string()),
        };

        let agent_id: Uuid = Uuid::new_v4();
        let subagent_id = agent_id.to_string();

        // Resolve the installed executor once. Classification happens
        // on the parent task before spawning so we don't spawn-and-
        // abandon on the background path.
        let exec = match get_subagent_executor() {
            Some(e) => e,
            None => {
                return ToolResult::error(
                    "subagent executor not installed — archon-core did not call \
                     install_subagent_executor before AgentTool::execute",
                );
            }
        };
        let classification = exec.classify(&request);

        // TASK-AGS-107: if the parent agent has a cancel_parent token,
        // create a child so cancelling the parent (Ctrl+C) cascades to
        // this subagent. Otherwise create a standalone token.
        let cancel = match &ctx.cancel_parent {
            Some(parent) => parent.child_token(),
            None => CancellationToken::new(),
        };
        let cancel_child = cancel.clone();
        // Kept alive after `cancel` is moved into the handle so the
        // register-failure branch below can still fire cancellation on
        // the already-spawned task.
        let cancel_for_failure = cancel.clone();
        let status: Arc<Mutex<AgentStatus>> = Arc::new(Mutex::new(AgentStatus::Running));
        let status_child = Arc::clone(&status);
        let result_slot = new_result_slot();
        let result_slot_child = Arc::clone(&result_slot);
        let ctx_clone = ctx.clone();
        let sid_spawn = subagent_id.clone();

        let join = tokio::spawn(async move {
            let outcome = run_subagent(sid_spawn.clone(), request, cancel_child, ctx_clone).await;
            let (final_status, payload) = match &outcome {
                SubagentOutcome::Completed(text) => (AgentStatus::Finished, Ok(text.clone())),
                SubagentOutcome::Failed(err) => (AgentStatus::Failed, Err(err.clone())),
                SubagentOutcome::AutoBackgrounded => {
                    // The runner is still executing — mark Running here
                    // so registry watchers don't see a premature
                    // terminal state. on_inner_complete will still fire
                    // from the runner's tail when it eventually finishes.
                    (
                        AgentStatus::Running,
                        Ok(format!("auto-backgrounded:{sid_spawn}")),
                    )
                }
                SubagentOutcome::Cancelled => {
                    (AgentStatus::Failed, Err("subagent cancelled".into()))
                }
            };
            *status_child
                .lock()
                .expect("status mutex poisoned in AgentTool::execute spawn") = final_status;
            *result_slot_child
                .lock()
                .expect("result_slot mutex poisoned in AgentTool::execute spawn") = Some(payload);
            outcome
        });

        // Background path: detach the JoinHandle (we can't await it from
        // here without blocking), register the handle with a placeholder
        // abort handle, and return the spawn marker. Use an adapter
        // tokio::spawn to give BackgroundAgentHandle a `JoinHandle<()>`
        // since the inner join returns `SubagentOutcome`.
        if matches!(classification, SubagentClassification::ExplicitBackground) {
            let adapter = tokio::spawn(async move {
                let _ = join.await;
            });

            // TASK-AGS-108 ERR-ARCH-01: keep a clone for retry on collision.
            let result_slot_retry = Arc::clone(&result_slot);
            let handle = BackgroundAgentHandle {
                agent_id,
                join_handle: Some(adapter),
                cancel_token: cancel,
                spawned_at: SystemTime::now(),
                status,
                result_slot,
            };

            // TASK-AGS-108 ERR-ARCH-01: retry-once on duplicate UUID collision.
            // If the astronomically-rare UUID collision hits, regenerate the
            // agent_id in the handle and retry once. On second collision,
            // surface the error and cancel the spawned task.
            match BACKGROUND_AGENTS.register(handle) {
                Ok(()) => {}
                Err(RegistryError::Duplicate(dup_id)) => {
                    tracing::warn!(
                        agent_id = %dup_id,
                        "Subagent ID collision: retrying with new UUID"
                    );
                    let new_id = Uuid::new_v4();
                    let retry_handle = BackgroundAgentHandle {
                        agent_id: new_id,
                        join_handle: None, // adapter already consumed; the task runs detached
                        cancel_token: cancel_for_failure.clone(),
                        spawned_at: SystemTime::now(),
                        status: Arc::new(Mutex::new(AgentStatus::Running)),
                        result_slot: result_slot_retry,
                    };
                    if let Err(e2) = BACKGROUND_AGENTS.register(retry_handle) {
                        cancel_for_failure.cancel();
                        return ToolResult::error(format!(
                            "background registry register failed after retry: {e2}"
                        ));
                    }
                }
                Err(e) => {
                    cancel_for_failure.cancel();
                    return ToolResult::error(format!("background registry register failed: {e}"));
                }
            }
            drop(cancel_for_failure);

            return ToolResult::success(
                json!({
                    "agent_id": agent_id.to_string(),
                    "status": "spawned",
                })
                .to_string(),
            );
        }

        // Foreground path: register the handle first (so parallel
        // tooling can observe the running agent), then await the join.
        // The join resolves with the final SubagentOutcome which we map
        // to a user-facing ToolResult.
        //
        // We cannot reuse the same JoinHandle for both registration and
        // the local .await, so we move the join into a oneshot by
        // splitting: the spawned task writes its terminal status via
        // `status_child` + `result_slot_child` (already wired above)
        // and we await the join ourselves below.
        let handle = {
            // Adapter JoinHandle<()> — we still want the registry to
            // own a clean Joinable handle even though the real outcome
            // is delivered via result_slot. For the foreground path we
            // don't actually need the registry lookup, but registering
            // is cheap and preserves symmetry with the background path.
            let (reg_cancel_tx, reg_cancel_rx) = tokio::sync::oneshot::channel::<()>();
            // Drop reg_cancel_tx on the happy path — we only use the rx
            // as an adapter target that never fires, keeping the adapter
            // task alive until the real join completes.
            drop(reg_cancel_tx);
            let reg_adapter = tokio::spawn(async move {
                let _ = reg_cancel_rx.await; // never resolves; task is idle
            });
            // Immediately abort the idle adapter — the foreground path
            // does not actually need it once we've awaited the real
            // outcome. We pre-register a nominal handle for symmetry.
            reg_adapter.abort();
            let noop_join: tokio::task::JoinHandle<()> = tokio::spawn(async {});

            BackgroundAgentHandle {
                agent_id,
                join_handle: Some(noop_join),
                cancel_token: cancel.clone(),
                spawned_at: SystemTime::now(),
                status: Arc::clone(&status),
                result_slot: Arc::clone(&result_slot),
            }
        };
        if let Err(e) = BACKGROUND_AGENTS.register(handle) {
            cancel_for_failure.cancel();
            return ToolResult::error(format!("background registry register failed: {e}"));
        }
        drop(cancel_for_failure);

        // Await the spawned `run_subagent` future. This is the
        // foreground contract: we block here until the executor either
        // completes, fails, auto-backgrounds (timer), or cancels.
        let outcome = match join.await {
            Ok(o) => o,
            Err(e) => {
                return ToolResult::error(format!("subagent join panicked: {e}"));
            }
        };

        match outcome {
            SubagentOutcome::Completed(text) => ToolResult::success(text),
            SubagentOutcome::Failed(err) => ToolResult::error(err),
            SubagentOutcome::AutoBackgrounded => {
                // Preserve the EXACT old text format from
                // agent.rs:3050-3053 so Sherlock's byte-for-byte checks
                // on the auto-background marker still pass.
                let ms = exec.auto_background_ms();
                let secs = if ms == 0 { 120 } else { ms / 1000 };
                ToolResult::success(format!(
                    "Subagent '{subagent_id}' auto-backgrounded after {secs}s. Still running — \
                     use SendMessage to check status."
                ))
            }
            SubagentOutcome::Cancelled => ToolResult::error("subagent cancelled"),
        }
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Risky
    }
}

// ---------------------------------------------------------------------------
// run_subagent — the AGT-025 `tokio::select!` race, relocated from
// archon-core per TASK-AGS-105 mapping doc Section 2c.
// ---------------------------------------------------------------------------
//
// Owns the AGT-025 auto-background race against the installed
// `SubagentExecutor`. The executor's `run_to_completion` fires
// `on_inner_complete` at its tail UNCONDITIONALLY (preserves
// PRESERVE-D8). `run_subagent` fires `on_visible_complete` only on the
// non-timer arms (preserves PRESERVE-D5 — post-abandonment auto-bg
// agents get inner side effects but NOT visible hooks).
pub async fn run_subagent(
    subagent_id: String,
    request: SubagentRequest,
    cancel: CancellationToken,
    ctx: ToolContext,
) -> SubagentOutcome {
    use std::time::Duration;

    let exec = match get_subagent_executor() {
        Some(e) => e,
        None => {
            return SubagentOutcome::Failed("subagent executor not installed".to_string());
        }
    };
    let auto_bg_ms = exec.auto_background_ms();

    let nested = ctx.nested;
    let join = tokio::spawn({
        let exec = Arc::clone(&exec);
        let cancel = cancel.clone();
        let ctx = ctx.clone();
        let req = request.clone();
        let sid = subagent_id.clone();
        async move { exec.run_to_completion(sid, req, ctx, cancel).await }
    });

    let outcome = if auto_bg_ms == 0 {
        tokio::select! {
            _ = cancel.cancelled() => SubagentOutcome::Cancelled,
            r = join => match r {
                Ok(Ok(text))  => SubagentOutcome::Completed(text),
                Ok(Err(e))    => SubagentOutcome::Failed(format!("{e}")),
                Err(e)        => SubagentOutcome::Failed(format!("join panic: {e}")),
            },
        }
    } else {
        let timer = tokio::time::sleep(Duration::from_millis(auto_bg_ms));
        tokio::select! {
            _ = cancel.cancelled() => SubagentOutcome::Cancelled,
            r = join => match r {
                Ok(Ok(text))  => SubagentOutcome::Completed(text),
                Ok(Err(e))    => SubagentOutcome::Failed(format!("{e}")),
                Err(e)        => SubagentOutcome::Failed(format!("join panic: {e}")),
            },
            _ = timer => SubagentOutcome::AutoBackgrounded,
        }
    };

    // on_visible_complete fires ONLY for non-timer completion arms.
    // The AutoBackgrounded arm INTENTIONALLY does NOT call it, which
    // preserves PRESERVE-D5: post-abandonment auto-backgrounded agents
    // get inner side effects (fired from run_to_completion's tail when
    // the runner eventually finishes) but NOT visible hooks or
    // worktree cleanup.
    match &outcome {
        SubagentOutcome::Completed(text) => {
            let side_effects = exec
                .on_visible_complete(subagent_id.clone(), Ok(text.clone()), nested)
                .await;
            // If there's a worktree-preserved note, splice it into the
            // returned text. The executor returned the base text via
            // run_to_completion; we append the suffix here so the
            // caller (AgentTool::execute) receives the fully-composed
            // string with no awareness of worktree plumbing.
            if let Some(suffix) = side_effects.text_suffix {
                return SubagentOutcome::Completed(format!("{text}{suffix}"));
            }
        }
        SubagentOutcome::Failed(err) => {
            let _ = exec
                .on_visible_complete(subagent_id.clone(), Err(err.clone()), nested)
                .await;
        }
        SubagentOutcome::AutoBackgrounded => {
            // NO on_visible_complete call — see PRESERVE-D5 above.
        }
        SubagentOutcome::Cancelled => {
            let _ = exec
                .on_visible_complete(
                    subagent_id.clone(),
                    Err("subagent cancelled".to_string()),
                    nested,
                )
                .await;
        }
    }

    outcome
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_ctx() -> ToolContext {
        ToolContext {
            working_dir: std::env::temp_dir(),
            session_id: "test-session".into(),
            mode: crate::tool::AgentMode::Normal,
            extra_dirs: vec![],
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn valid_input_returns_subagent_request() {
        // TASK-AGS-104: execute() now returns {agent_id,status}; validate
        // SubagentRequest shape directly via validate_and_build.
        let tool = AgentTool::new();
        let input = json!({
            "prompt": "Summarize the codebase",
            "model": "claude-sonnet-4-6",
            "allowed_tools": ["Read", "Glob"],
            "max_turns": 5
        });

        let request = tool.validate_and_build(&input).expect("valid input");
        assert_eq!(request.prompt, "Summarize the codebase");
        assert_eq!(request.model.as_deref(), Some("claude-sonnet-4-6"));
        assert_eq!(request.allowed_tools, vec!["Read", "Glob"]);
        assert_eq!(request.max_turns, 5);
        assert_eq!(request.timeout_secs, 300);
        assert!(!request.run_in_background);
        assert!(request.cwd.is_none());
    }

    #[tokio::test]
    async fn missing_prompt_returns_error() {
        let tool = AgentTool::new();
        let input = json!({ "model": "claude-sonnet-4-6" });

        let result = tool.execute(input, &make_ctx()).await;
        assert!(result.is_error);
        assert!(
            result.content.contains("prompt"),
            "error should mention 'prompt': {}",
            result.content
        );
    }

    #[tokio::test]
    async fn empty_prompt_returns_error() {
        let tool = AgentTool::new();
        let input = json!({ "prompt": "   " });

        let result = tool.execute(input, &make_ctx()).await;
        assert!(result.is_error);
        assert!(result.content.contains("prompt"));
    }

    #[tokio::test]
    async fn default_max_turns_applied() {
        let tool = AgentTool::new();
        let input = json!({ "prompt": "Do something" });

        let request = tool.validate_and_build(&input).expect("valid input");
        assert_eq!(request.max_turns, SubagentRequest::DEFAULT_MAX_TURNS);
        assert_eq!(request.timeout_secs, SubagentRequest::DEFAULT_TIMEOUT_SECS);
        assert!(!request.run_in_background);
        assert!(request.cwd.is_none());
    }

    #[tokio::test]
    async fn allowed_tools_parsed_from_array() {
        let tool = AgentTool::new();
        let input = json!({
            "prompt": "Refactor module",
            "allowed_tools": ["Read", "Write", "Edit"]
        });

        let request = tool.validate_and_build(&input).expect("valid input");
        assert_eq!(request.allowed_tools, vec!["Read", "Write", "Edit"]);
    }

    #[tokio::test]
    async fn no_allowed_tools_gives_empty_vec() {
        let tool = AgentTool::new();
        let input = json!({ "prompt": "Analyze code" });

        let request = tool.validate_and_build(&input).expect("valid input");
        assert!(request.allowed_tools.is_empty());
    }

    #[tokio::test]
    async fn invalid_max_turns_returns_error() {
        let tool = AgentTool::new();

        // Zero
        let result = tool
            .execute(json!({"prompt": "x", "max_turns": 0}), &make_ctx())
            .await;
        assert!(result.is_error);

        // Over 100
        let result = tool
            .execute(json!({"prompt": "x", "max_turns": 101}), &make_ctx())
            .await;
        assert!(result.is_error);
    }

    #[test]
    fn permission_level_is_risky() {
        let tool = AgentTool::new();
        assert_eq!(tool.permission_level(&json!({})), PermissionLevel::Risky);
    }

    #[tokio::test]
    async fn subagent_type_parsed_when_present() {
        let tool = AgentTool::new();
        let input = json!({
            "prompt": "Review code",
            "subagent_type": "code-reviewer"
        });

        let request = tool.validate_and_build(&input).expect("valid input");
        assert_eq!(request.subagent_type.as_deref(), Some("code-reviewer"));
    }

    #[tokio::test]
    async fn subagent_type_none_when_absent() {
        let tool = AgentTool::new();
        let input = json!({ "prompt": "Do something" });

        let request = tool.validate_and_build(&input).expect("valid input");
        assert!(request.subagent_type.is_none());
    }

    #[test]
    fn subagent_type_backward_compatible_deserialization() {
        // JSON without subagent_type should deserialize fine (serde default)
        let json = r#"{
            "prompt": "test",
            "allowed_tools": [],
            "max_turns": 10,
            "timeout_secs": 300
        }"#;
        let request: SubagentRequest = serde_json::from_str(json).unwrap();
        assert!(request.subagent_type.is_none());
    }

    #[test]
    fn subagent_type_serializes_to_json() {
        let request = SubagentRequest {
            prompt: "test".into(),
            model: None,
            allowed_tools: vec![],
            max_turns: 10,
            timeout_secs: 300,
            subagent_type: Some("code-reviewer".into()),
            run_in_background: false,
            cwd: None,
            isolation: None,
        };
        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["subagent_type"], "code-reviewer");
    }

    #[test]
    fn schema_includes_subagent_type() {
        let tool = AgentTool::new();
        let schema = tool.input_schema();
        let props = schema["properties"].as_object().unwrap();
        assert!(props.contains_key("subagent_type"));
        assert_eq!(props["subagent_type"]["type"], "string");
    }

    #[tokio::test]
    async fn run_in_background_parsed_when_present() {
        let tool = AgentTool::new();
        let input = json!({
            "prompt": "Review code",
            "run_in_background": true
        });

        let request = tool.validate_and_build(&input).expect("valid input");
        assert!(request.run_in_background);
    }

    #[test]
    fn run_in_background_defaults_to_false() {
        let json = r#"{
            "prompt": "test",
            "allowed_tools": [],
            "max_turns": 10,
            "timeout_secs": 300
        }"#;
        let request: SubagentRequest = serde_json::from_str(json).unwrap();
        assert!(!request.run_in_background);
    }

    #[test]
    fn run_in_background_serializes_to_json() {
        let request = SubagentRequest {
            prompt: "test".into(),
            model: None,
            allowed_tools: vec![],
            max_turns: 10,
            timeout_secs: 300,
            subagent_type: None,
            run_in_background: true,
            cwd: None,
            isolation: None,
        };
        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["run_in_background"], true);
    }

    #[tokio::test]
    async fn cwd_parsed_when_present() {
        let tool = AgentTool::new();
        let input = json!({
            "prompt": "Review code",
            "cwd": "/tmp"
        });

        let request = tool.validate_and_build(&input).expect("valid input");
        assert_eq!(request.cwd.as_deref(), Some("/tmp"));
    }

    #[test]
    fn cwd_defaults_to_none() {
        let json = r#"{
            "prompt": "test",
            "allowed_tools": [],
            "max_turns": 10,
            "timeout_secs": 300
        }"#;
        let request: SubagentRequest = serde_json::from_str(json).unwrap();
        assert!(request.cwd.is_none());
    }

    #[test]
    fn cwd_serializes_to_json() {
        let request = SubagentRequest {
            prompt: "test".into(),
            model: None,
            allowed_tools: vec![],
            max_turns: 10,
            timeout_secs: 300,
            subagent_type: None,
            run_in_background: false,
            cwd: Some("/tmp".into()),
            isolation: None,
        };
        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["cwd"], "/tmp");
    }

    // -----------------------------------------------------------------------
    // Worktree isolation tests (AGT-017)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn isolation_worktree_parsed_when_present() {
        let tool = AgentTool::new();
        let input = json!({
            "prompt": "Review code",
            "isolation": "worktree"
        });

        let request = tool.validate_and_build(&input).expect("valid input");
        assert_eq!(request.isolation.as_deref(), Some("worktree"));
    }

    #[tokio::test]
    async fn isolation_none_when_absent() {
        let tool = AgentTool::new();
        let input = json!({ "prompt": "Do something" });

        let request = tool.validate_and_build(&input).expect("valid input");
        assert!(request.isolation.is_none());
    }

    #[test]
    fn isolation_backward_compatible_deserialization() {
        let json = r#"{
            "prompt": "test",
            "allowed_tools": [],
            "max_turns": 10,
            "timeout_secs": 300
        }"#;
        let request: SubagentRequest = serde_json::from_str(json).unwrap();
        assert!(request.isolation.is_none());
    }

    #[test]
    fn isolation_serializes_to_json() {
        let request = SubagentRequest {
            prompt: "test".into(),
            model: None,
            allowed_tools: vec![],
            max_turns: 10,
            timeout_secs: 300,
            subagent_type: None,
            run_in_background: false,
            cwd: None,
            isolation: Some("worktree".into()),
        };
        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["isolation"], "worktree");
    }

    #[test]
    fn schema_includes_isolation() {
        let tool = AgentTool::new();
        let schema = tool.input_schema();
        let props = schema["properties"].as_object().unwrap();
        assert!(props.contains_key("isolation"));
        assert_eq!(props["isolation"]["type"], "string");
    }

    #[test]
    fn schema_includes_run_in_background_and_cwd() {
        let tool = AgentTool::new();
        let schema = tool.input_schema();
        let props = schema["properties"].as_object().unwrap();
        assert!(props.contains_key("run_in_background"));
        assert_eq!(props["run_in_background"]["type"], "boolean");
        assert!(props.contains_key("cwd"));
        assert_eq!(props["cwd"]["type"], "string");
    }

    #[test]
    fn tool_metadata() {
        let tool = AgentTool::new();
        assert_eq!(tool.name(), "Agent");
        assert!(!tool.description().is_empty());

        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        assert!(
            schema["required"]
                .as_array()
                .unwrap()
                .contains(&json!("prompt"))
        );
    }
}
