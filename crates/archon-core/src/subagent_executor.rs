//! TASK-AGS-105: `AgentSubagentExecutor` — the archon-core side of the
//! `SubagentExecutor` trait seam defined in `archon_tools::subagent_executor`.
//!
//! This is where the 766-line body of the old
//! `Agent::handle_subagent_result` helper lives in the TASK-AGS-105
//! world. The old helper was removed from `agent.rs` (the legacy spawn
//! block at lines 2939-2977 and the body at lines 2411-3176). The
//! dispatch loop and the SendMessage resume path now call
//! `archon_tools::agent_tool::run_subagent` directly, which calls into
//! this executor via the process-global OnceLock registry.
//!
//! Lifecycle (maps Section 3 of docs/task-ags-105-mapping.md):
//!
//! - `classify(&request)`: decides Foreground vs ExplicitBackground
//!   based on `request.run_in_background`, the agent definition's
//!   `background` flag (resolved via the registry), and the
//!   `ARCHON_FORK_*` env toggle. Called by `AgentTool::execute` BEFORE
//!   spawning `run_subagent`.
//! - `run_to_completion(...)`: the big one. Fires `SubagentStart` +
//!   `TaskCreated` at the top, runs the early-return guards, builds
//!   the runner, runs it to completion, and at the tail calls
//!   `on_inner_complete` UNCONDITIONALLY (preserves PRESERVE-D8 — the
//!   single save_agent_memory site, collapsed from 3 old sites).
//! - `on_visible_complete(...)`: fires hooks and cleans up worktrees.
//!   Only called from `run_subagent`'s non-timer completion arms.
//!
//! PRESERVE notes:
//! - PRESERVE-D5: on the `AutoBackgrounded` timer arm, `run_subagent`
//!   does NOT call `on_visible_complete`. The runner task continues
//!   executing in its own tokio task; when it eventually finishes,
//!   `run_to_completion`'s tail fires `on_inner_complete` from that
//!   orphaned task. Hooks + worktree cleanup never fire.
//! - PRESERVE-D8: `save_agent_memory` is called from exactly ONE place
//!   in the new code — inside `on_inner_complete`. Verified by
//!   `grep -n save_agent_memory crates/archon-core/src/subagent_executor*`
//!   returning a single hit inside `subagent_executor/completion.rs`.
//!   Old M1/M2/M3 collapsed.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use archon_llm::identity::IdentityProvider;
use archon_llm::provider::LlmProvider;
use archon_memory::MemoryTrait;
use archon_tools::agent_tool::SubagentRequest;
use archon_tools::subagent_executor::{
    ExecutorError, OutcomeSideEffects, SubagentClassification, SubagentExecutor,
};
use archon_tools::tool::ToolContext;
use archon_tools::worktree_manager::WorktreeInfo;

use crate::agents::AgentRegistry;
use crate::agents::CustomAgentDefinition;
use crate::dispatch::ToolRegistry;
use crate::hooks::{HookEvent, HookRegistry};
use crate::subagent::SubagentManager;

mod activity;
mod classification;
mod completion;
mod run;

/// Snapshot of the `Agent` fields that the executor needs.
///
/// This is populated by `Agent::new` and installed into the process
/// OnceLock via `archon_tools::subagent_executor::install_subagent_executor`.
pub struct AgentSubagentExecutor {
    client: Arc<dyn LlmProvider>,
    tool_registry: ToolRegistry,
    subagent_manager: Arc<Mutex<SubagentManager>>,
    agent_registry: Arc<std::sync::RwLock<AgentRegistry>>,
    hook_registry: Option<Arc<HookRegistry>>,
    memory: Option<Arc<dyn MemoryTrait>>,
    /// Parent `AgentConfig.working_dir` (used as project_path +
    /// fallback CWD when neither worktree nor request.cwd overrides).
    working_dir: std::path::PathBuf,
    /// Parent `AgentConfig.session_id` (used for hook firing +
    /// session-scoped hook registration).
    session_id: String,
    /// Parent model (used as the fallback in the model resolution
    /// chain: request → definition → parent).
    parent_model: String,
    /// Parent `system_prompt` (used for fork-agent parent context
    /// inheritance at the 50KB truncation).
    parent_system_prompt: Vec<serde_json::Value>,
    /// Parent permission mode (used in the subagent_mode resolution
    /// cascade).
    parent_permission_mode: Arc<Mutex<String>>,
    /// Shared pending resume messages slot (written from
    /// `Agent::process_message` SendMessage resume path, read from
    /// `run_to_completion` when building the runner).
    pending_resume_messages: Arc<Mutex<Option<Vec<serde_json::Value>>>>,
    /// Parent AgentConfig for structural LLM request field alignment
    /// (max_tokens, thinking, speed, effort live reads at subagent build time).
    agent_config: Arc<crate::agent::AgentConfig>,
    /// Parent identity provider for billing-header prepend in spoof mode
    /// (v0.1.19 — subagent system prompt alignment with parent's).
    identity: Arc<IdentityProvider>,
    /// Per-subagent worktree info cache. Populated inside
    /// `run_to_completion` when `isolation == "worktree"`; consumed by
    /// `on_visible_complete` when deciding clean-vs-preserved worktree
    /// cleanup. The entry is removed from the map after cleanup so
    /// successive runs with the same id don't see stale data.
    worktree_cache: Arc<Mutex<HashMap<String, WorktreeInfo>>>,
    /// Per-subagent agent-type / memory metadata cache. Populated in
    /// `run_to_completion` and consumed by `on_inner_complete` when
    /// deciding whether to call `save_agent_memory`.
    memory_cache: Arc<Mutex<HashMap<String, MemoryMeta>>>,
}

/// Per-subagent metadata the executor caches between
/// `run_to_completion` (where it resolves the agent def) and
/// `on_inner_complete` (where it calls `save_agent_memory`).
#[derive(Debug, Clone)]
struct MemoryMeta {
    agent_type: Option<String>,
    memory_scope: Option<crate::agents::AgentMemoryScope>,
    tags: Vec<String>,
}

impl AgentSubagentExecutor {
    /// Construct a new executor from the relevant `Agent` fields.
    ///
    /// The `pending_resume_messages` slot is shared with the parent
    /// `Agent` so the SendMessage resume path can stash messages into
    /// it without crossing the executor boundary.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        client: Arc<dyn LlmProvider>,
        tool_registry: ToolRegistry,
        subagent_manager: Arc<Mutex<SubagentManager>>,
        agent_registry: Arc<std::sync::RwLock<AgentRegistry>>,
        hook_registry: Option<Arc<HookRegistry>>,
        memory: Option<Arc<dyn MemoryTrait>>,
        working_dir: std::path::PathBuf,
        session_id: String,
        parent_model: String,
        parent_system_prompt: Vec<serde_json::Value>,
        parent_permission_mode: Arc<Mutex<String>>,
        pending_resume_messages: Arc<Mutex<Option<Vec<serde_json::Value>>>>,
        agent_config: Arc<crate::agent::AgentConfig>,
        identity: Arc<IdentityProvider>,
    ) -> Self {
        Self {
            client,
            tool_registry,
            subagent_manager,
            agent_registry,
            hook_registry,
            memory,
            working_dir,
            session_id,
            parent_model,
            parent_system_prompt,
            parent_permission_mode,
            pending_resume_messages,
            agent_config,
            identity,
            worktree_cache: Arc::new(Mutex::new(HashMap::new())),
            memory_cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Fire a hook via the optional hook registry. Inlined from
    /// `Agent::fire_hook` per mapping doc Section 7-Q4: plain helper,
    /// no HookFirer trait. No-op when no registry is set.
    async fn fire_hook(&self, event: HookEvent, payload: serde_json::Value) {
        if let Some(ref registry) = self.hook_registry {
            registry
                .execute_hooks(event, payload, &self.working_dir, &self.session_id)
                .await;
        }
    }

    /// Build the filtered tool registry for a subagent. Ported from
    /// the old `Agent::build_subagent_tools` at `agent.rs:3182` — see
    /// mapping doc Section 7-Q5 (option b: method on the executor).
    pub async fn build_subagent_tools(
        &self,
        request: &SubagentRequest,
        agent_def: Option<&CustomAgentDefinition>,
    ) -> (Vec<serde_json::Value>, ToolRegistry) {
        // Hardcoded denylist — subagents must NEVER have these tools
        const DENYLIST: &[&str] = &[
            "Agent",
            "AskUserQuestion",
            "EnterPlanMode",
            "ExitPlanMode",
            "TaskCreate",
            "TaskStop",
        ];

        const DEFAULT_TOOLS: &[&str] = &["Read", "Grep", "Glob", "Bash", "Write", "Edit"];

        let base_allowed: Vec<&str> = if !request.allowed_tools.is_empty() {
            request
                .allowed_tools
                .iter()
                .map(|s| s.as_str())
                .filter(|n| !DENYLIST.contains(n))
                .collect()
        } else if let Some(def_tools) = agent_def.and_then(|d| d.allowed_tools.as_ref()) {
            def_tools
                .iter()
                .map(|s| s.as_str())
                .filter(|n| !DENYLIST.contains(n))
                .collect()
        } else {
            DEFAULT_TOOLS.to_vec()
        };

        let agent_deny: Vec<String> = agent_def
            .and_then(|d| d.disallowed_tools.as_ref())
            .cloned()
            .unwrap_or_default();

        const PLAN_MODE_DENY: &[&str] = &["Write", "Edit", "Bash", "NotebookEdit"];
        let is_plan_mode = self.parent_permission_mode.lock().await.as_str() == "plan";

        let mcp_scope: Option<&Vec<String>> = agent_def.and_then(|d| d.mcp_servers.as_ref());

        let final_allowed: Vec<&str> = base_allowed
            .into_iter()
            .filter(|n| !agent_deny.iter().any(|d| d == n))
            .filter(|n| !is_plan_mode || !PLAN_MODE_DENY.contains(n))
            .filter(|n| {
                if let Some(allowed_servers) = mcp_scope
                    && n.starts_with("mcp__")
                {
                    let parts: Vec<&str> = n.splitn(3, "__").collect();
                    if parts.len() >= 2 {
                        let server = parts[1];
                        return allowed_servers
                            .iter()
                            .any(|s| s.eq_ignore_ascii_case(server));
                    }
                    return false;
                }
                true
            })
            .collect();

        let filtered = self.tool_registry.clone_filtered(&final_allowed);
        let defs = filtered.tool_definitions();
        (defs, filtered)
    }
}

#[async_trait]
impl SubagentExecutor for AgentSubagentExecutor {
    fn auto_background_ms(&self) -> u64 {
        crate::subagent::get_auto_background_ms()
    }

    fn classify(&self, request: &SubagentRequest) -> SubagentClassification {
        self.classify_request(request)
    }

    async fn run_to_completion(
        &self,
        subagent_id: String,
        request: SubagentRequest,
        ctx: ToolContext,
        cancel: CancellationToken,
    ) -> Result<String, ExecutorError> {
        self.run_subagent_to_completion(subagent_id, request, ctx, cancel)
            .await
    }

    async fn on_inner_complete(&self, subagent_id: String, result: Result<String, String>) {
        self.handle_inner_complete(subagent_id, result).await;
    }

    async fn on_visible_complete(
        &self,
        subagent_id: String,
        result: Result<String, String>,
        nested: bool,
    ) -> OutcomeSideEffects {
        self.handle_visible_complete(subagent_id, result, nested)
            .await
    }
}
