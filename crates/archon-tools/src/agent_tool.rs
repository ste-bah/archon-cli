use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio_util::sync::CancellationToken;
use tracing::{error, warn};
use uuid::Uuid;

use crate::background_agents::{
    AgentStatus, BACKGROUND_AGENTS, BackgroundAgentHandle, RegistryError, new_result_slot,
};
use crate::subagent_executor::{SubagentClassification, SubagentOutcome, get_subagent_executor};
use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

const INLINE_AGENT_LIMIT: usize = 20;
const AGENT_DESCRIPTION_LIMIT_BYTES: usize = 4096;
const CATALOG_PAGE_LIMIT: usize = 25;

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
    ///
    /// Effectively unlimited. archon trusts the configured LLM provider to
    /// return results and errors; runaway-loop protection is the USD budget
    /// cap (`--max-budget-usd`), not an arbitrary turn count. The hard
    /// ceiling exists only to bound the integer type and keep tests
    /// deterministic — no realistic agent run hits it.
    pub const DEFAULT_MAX_TURNS: u32 = Self::MAX_TURNS_HARD_CAP;

    /// Hard upper bound for `max_turns`. Effectively unlimited — set
    /// high enough that no realistic agent run will hit it. Runaway-loop
    /// protection is the budget cap (`--max-budget-usd`), not this.
    pub const MAX_TURNS_HARD_CAP: u32 = 100_000;

    /// Default timeout in seconds — 24 hours.
    ///
    /// Was 300s (5 min). That was a footgun for survey/analysis agents
    /// that legitimately need 10-30+ minutes to scan large codebases and
    /// write structured findings to memory. Bumped to effectively-
    /// unlimited because we already have a real runaway guard
    /// (`--max-budget-usd`) and the wall-clock cap is double-jeopardy
    /// that punishes correct behavior (slow because reading a lot)
    /// without preventing the actual failure mode (stuck-in-a-loop).
    pub const DEFAULT_TIMEOUT_SECS: u64 = 86_400;
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
// Failure classification — additive prefix so the LLM stops guessing
// "rate limited" when the real error is something else.
// ---------------------------------------------------------------------------

/// Conservative heuristic: classify a subagent failure string into a
/// category prefix. Only emits a specific category when the signal is
/// unambiguous (HTTP status codes, exact Rust panic format, etc.).
/// Defaults to neutral `[subagent_failure]` — the original error text
/// carries the truth regardless.
pub fn classify_failure_prefix(err: &str) -> &'static str {
    let low = err.to_lowercase();

    // Rate limit: requires HTTP 429 OR the explicit phrase "rate limit"
    // surrounded by word boundaries.
    if low.contains("429 ")
        || low.contains(" 429")
        || low == "429"
        || low.contains(" rate limit ")
        || low.starts_with("rate limit ")
        || low.ends_with(" rate limit")
        || low.contains("rate-limit")
    {
        return "[subagent_rate_limited]";
    }

    // Auth: HTTP 401 OR explicit "authentication failed" / "invalid api key" /
    // "unauthorized". Do NOT match generic "auth" substring.
    if low.contains(" 401")
        || low.contains("401 ")
        || low.contains("authentication failed")
        || low.contains("invalid api key")
        || low.contains("unauthorized")
    {
        return "[subagent_auth_failed]";
    }

    // Panic: only the explicit "panicked at" phrase (standard Rust format).
    if low.contains("panicked at") || low.contains("thread '") {
        return "[subagent_panic]";
    }

    // Timeout: "timed out", "timeout exceeded", "deadline exceeded".
    if low.contains("timed out")
        || low.contains("timeout exceeded")
        || low.contains("deadline exceeded")
    {
        return "[subagent_timeout]";
    }

    // Default — the error text carries the truth; we just label it
    // generically so the LLM knows it was a subagent failure.
    "[subagent_failure]"
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
            tool set. Use known subagent_type names directly. Use AgentCatalog to list, search, \
            or inspect less-common agents before launching them."
                .to_string();

        if !agents.is_empty() {
            desc.push_str("\n\nCommon agents: ");
            let entries: Vec<String> = agents
                .iter()
                .take(INLINE_AGENT_LIMIT)
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

        if desc.len() > AGENT_DESCRIPTION_LIMIT_BYTES {
            desc.truncate(AGENT_DESCRIPTION_LIMIT_BYTES);
        }

        Self { description: desc }
    }
}

pub struct AgentCatalogTool {
    agents: Vec<(String, String)>,
}

impl AgentCatalogTool {
    pub fn new(mut agents: Vec<(String, String)>) -> Self {
        agents.sort_by(|a, b| a.0.cmp(&b.0));
        Self { agents }
    }

    fn capped_limit(input: &serde_json::Value) -> usize {
        input
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| n.clamp(1, CATALOG_PAGE_LIMIT as u64) as usize)
            .unwrap_or(10)
    }

    fn entry_json((name, summary): &(String, String)) -> serde_json::Value {
        json!({
            "name": name,
            "description": summary,
        })
    }

    fn list(&self, input: &serde_json::Value) -> serde_json::Value {
        let limit = Self::capped_limit(input);
        let page = input.get("page").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let start = page.saturating_mul(limit);
        let agents: Vec<_> = self
            .agents
            .iter()
            .skip(start)
            .take(limit)
            .map(Self::entry_json)
            .collect();
        json!({
            "action": "list",
            "page": page,
            "limit": limit,
            "total": self.agents.len(),
            "agents": agents,
        })
    }

    fn search(&self, input: &serde_json::Value) -> serde_json::Value {
        let limit = Self::capped_limit(input);
        let query = input
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let agents: Vec<_> = self
            .agents
            .iter()
            .filter(|(name, summary)| {
                let name = name.to_ascii_lowercase();
                let summary = summary.to_ascii_lowercase();
                query.is_empty() || name.contains(&query) || summary.contains(&query)
            })
            .take(limit)
            .map(Self::entry_json)
            .collect();
        json!({ "action": "search", "query": query, "limit": limit, "agents": agents })
    }

    fn info(&self, input: &serde_json::Value) -> Result<serde_json::Value, AgentToolError> {
        let name = input
            .get("name")
            .or_else(|| input.get("subagent_type"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .ok_or(AgentToolError::MissingField("name"))?;
        let Some(entry) = self.agents.iter().find(|(agent, _)| agent == name) else {
            return Err(AgentToolError::InvalidInput(format!(
                "unknown agent '{name}'"
            )));
        };
        Ok(json!({ "action": "info", "agent": Self::entry_json(entry) }))
    }
}

#[async_trait::async_trait]
impl Tool for AgentCatalogTool {
    fn name(&self) -> &str {
        "AgentCatalog"
    }

    fn description(&self) -> &str {
        "List, search, and inspect available subagent types. Use this for agent discovery; use the Agent tool to launch a known subagent_type."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "search", "info"],
                    "description": "Catalog action to run."
                },
                "query": {
                    "type": "string",
                    "description": "Search text for action=search."
                },
                "name": {
                    "type": "string",
                    "description": "Agent type name for action=info."
                },
                "page": {
                    "type": "integer",
                    "description": "Zero-based page for action=list."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum rows to return, capped by Archon."
                }
            }
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let action = input
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("list");
        let result = match action {
            "list" => Ok(self.list(&input)),
            "search" => Ok(self.search(&input)),
            "info" => self.info(&input),
            other => Err(AgentToolError::InvalidInput(format!(
                "unknown AgentCatalog action '{other}'"
            ))),
        };
        match result {
            Ok(value) => ToolResult::success(value.to_string()),
            Err(err) => ToolResult::error(err.to_string()),
        }
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
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
                let cap = u64::from(SubagentRequest::MAX_TURNS_HARD_CAP);
                if n == 0 || n > cap {
                    Err(AgentToolError::InvalidInput(format!(
                        "max_turns must be between 1 and {cap}"
                    )))
                } else {
                    warn!(
                        value = n,
                        tool = "Agent",
                        "max_turns emitted by model despite schema removal -- investigate"
                    );
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
        let subagent_type = request
            .subagent_type
            .clone()
            .unwrap_or_else(|| "default".to_string());

        let join = archon_observability::spawn_named(
            format!("subagent-runner:{subagent_type}"),
            async move {
                let outcome =
                    run_subagent(sid_spawn.clone(), request, cancel_child, ctx_clone).await;
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
                    .expect("result_slot mutex poisoned in AgentTool::execute spawn") =
                    Some(payload);
                outcome
            },
        );

        // Background path: detach the JoinHandle (we can't await it from
        // here without blocking), register the handle with a placeholder
        // abort handle, and return the spawn marker. Use an adapter
        // tokio::spawn to give BackgroundAgentHandle a `JoinHandle<()>`
        // since the inner join returns `SubagentOutcome`.
        if matches!(classification, SubagentClassification::ExplicitBackground) {
            let adapter =
                archon_observability::spawn_named("subagent-background-adapter", async move {
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
            let reg_adapter =
                archon_observability::spawn_named("subagent-registry-adapter", async move {
                    let _ = reg_cancel_rx.await; // never resolves; task is idle
                });
            // Immediately abort the idle adapter — the foreground path
            // does not actually need it once we've awaited the real
            // outcome. We pre-register a nominal handle for symmetry.
            reg_adapter.abort();
            let noop_join: tokio::task::JoinHandle<()> =
                archon_observability::spawn_named("subagent-noop-registration", async {});

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
            let msg = format!("background registry register failed: {e}");
            return ToolResult::error(format!("{} {}", classify_failure_prefix(&msg), msg));
        }
        drop(cancel_for_failure);

        // Await the spawned `run_subagent` future. This is the
        // foreground contract: we block here until the executor either
        // completes, fails, auto-backgrounds (timer), or cancels.
        let outcome = match join.await {
            Ok(o) => o,
            Err(e) => {
                error!(
                    subagent_id = %subagent_id,
                    subagent_type = ?subagent_type,
                    error = %e,
                    "AgentTool: subagent join panicked",
                );
                let msg = format!("subagent join panicked: {e}");
                return ToolResult::error(format!("{} {}", classify_failure_prefix(&msg), msg));
            }
        };

        match outcome {
            SubagentOutcome::Completed(text) => ToolResult::success(text),
            SubagentOutcome::Failed(err) => {
                error!(
                    subagent_id = %subagent_id,
                    subagent_type = ?subagent_type,
                    error = %err,
                    "AgentTool: subagent run failed",
                );
                let prefixed = format!("{} {}", classify_failure_prefix(&err), err);
                ToolResult::error(prefixed)
            }
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
            SubagentOutcome::Cancelled => {
                warn!(
                    subagent_id = %subagent_id,
                    "AgentTool: subagent cancelled",
                );
                let msg = "subagent cancelled".to_string();
                ToolResult::error(format!("{} {}", classify_failure_prefix(&msg), msg))
            }
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
    let join = archon_observability::spawn_named("subagent-executor", {
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
        assert_eq!(request.timeout_secs, SubagentRequest::DEFAULT_TIMEOUT_SECS);
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
    async fn model_omitting_max_turns_uses_default() {
        let tool = AgentTool::new();
        let request = tool
            .validate_and_build(&json!({"prompt": "x"}))
            .expect("default applies");
        assert_eq!(request.max_turns, SubagentRequest::DEFAULT_MAX_TURNS);
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

        // Over MAX_TURNS_HARD_CAP (100_000)
        let result = tool
            .execute(json!({"prompt": "x", "max_turns": 100_001}), &make_ctx())
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

    #[test]
    fn agent_tool_schema_does_not_expose_max_turns() {
        let tool = AgentTool::new();
        let schema = tool.input_schema();
        let props = schema["properties"].as_object().expect("properties");
        assert!(
            !props.contains_key("max_turns"),
            "AgentTool schema must not advertise max_turns"
        );
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
    fn agent_listing_is_capped_and_description_bounded() {
        let agents: Vec<_> = (0..30)
            .map(|idx| {
                (
                    format!("agent-{idx:02}"),
                    "long description that should not leak the whole catalog".repeat(20),
                )
            })
            .collect();

        let tool = AgentTool::with_agent_listing(&agents);

        assert!(tool.description().len() <= AGENT_DESCRIPTION_LIMIT_BYTES);
        assert!(tool.description().contains("agent-00"));
        assert!(tool.description().contains("AgentCatalog"));
        assert!(!tool.description().contains("agent-29"));
    }

    #[test]
    fn agent_catalog_lists_searches_and_infos_sorted_agents() {
        let tool = AgentCatalogTool::new(vec![
            ("zeta".into(), "last".into()),
            ("sherlock-holmes".into(), "forensic reviewer".into()),
            ("builder".into(), "implementation agent".into()),
        ]);

        let listed = tool.list(&json!({"action": "list", "limit": 2}));
        let listed_agents = listed["agents"].as_array().unwrap();
        assert_eq!(listed["total"], 3);
        assert_eq!(listed_agents[0]["name"], "builder");
        assert_eq!(listed_agents[1]["name"], "sherlock-holmes");

        let searched = tool.search(&json!({"action": "search", "query": "forensic"}));
        assert_eq!(searched["agents"][0]["name"], "sherlock-holmes");

        let info = tool.info(&json!({"name": "zeta"})).unwrap();
        assert_eq!(info["agent"]["description"], "last");
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
