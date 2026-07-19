use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use serde_json::json;
use tokio_util::sync::CancellationToken;
use tracing::{error, warn};
use uuid::Uuid;

use super::failure::classify_failure_prefix;
use super::request::AgentToolError;
use super::request::{expected_target_files, validate_and_build};
use super::run::run_subagent;
use crate::agent_mutation_guard::{snapshot_expected_targets, verify_expected_mutations};
use crate::background_agents::{
    AgentStatus, BACKGROUND_AGENTS, BackgroundAgentHandle, RegistryError, new_result_slot,
};
use crate::subagent_executor::{SubagentClassification, SubagentOutcome, get_subagent_executor};
use crate::subagent_request::SubagentRequest;
use crate::tool::{PermissionLevel, Tool, ToolContext, ToolResult};

const INLINE_AGENT_LIMIT: usize = 20;
pub(crate) const AGENT_DESCRIPTION_LIMIT_BYTES: usize = 4096;

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
                tool set. Use normal isolation for read-only work; only request worktree isolation \
                when the subagent needs isolated file edits."
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
            or inspect less-common agents before launching them. Use normal isolation for read-only \
            work; only request worktree isolation when the subagent needs isolated file edits."
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

impl Default for AgentTool {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentTool {
    pub(super) fn validate_and_build(
        &self,
        input: &serde_json::Value,
    ) -> Result<SubagentRequest, AgentToolError> {
        validate_and_build(input)
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
                    "description": "Optional model override. Omit this unless the user explicitly asks for a different model; omitted or empty inherits the parent model/provider. Do not invent provider model IDs."
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
                    "enum": ["none", "worktree"],
                    "description": "Optional isolation mode. Use 'none' or omit this field for normal/read-only subagents. Use 'worktree' only when isolated file edits are required."
                },
                "expected_target_files": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional file paths that must be changed by a foreground mutating subagent. Archon snapshots these paths before launch and fails the Agent result if they are unchanged after completion."
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
        let expected_target_files = match expected_target_files(&input) {
            Ok(paths) => paths,
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
        if !expected_target_files.is_empty()
            && matches!(classification, SubagentClassification::ExplicitBackground)
        {
            return ToolResult::error(
                "expected_target_files can only be verified for foreground subagents; \
                 remove run_in_background or omit expected_target_files",
            );
        }
        let expected_mutations =
            match snapshot_expected_targets(&expected_target_files, request.cwd.as_deref(), ctx) {
                Ok(snapshots) => snapshots,
                Err(err) => return ToolResult::error(err),
            };

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
            SubagentOutcome::Completed(text) => {
                match verify_expected_mutations(&expected_mutations) {
                    Ok(()) => ToolResult::success(text),
                    Err(err) => {
                        ToolResult::error(format!("[subagent_expected_mutation_missing] {err}"))
                    }
                }
            }
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
                if !expected_mutations.is_empty() {
                    return ToolResult::error(format!(
                        "[subagent_mutation_unverified] Subagent '{subagent_id}' \
                         auto-backgrounded after {secs}s before expected file mutations could be verified. \
                         It is still running; inspect it with SendMessage before trusting the edit."
                    ));
                }
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
