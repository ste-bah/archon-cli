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
use tokio::sync::{Mutex, Semaphore};
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
mod paths;
mod run;
mod run_prepare;
mod run_runner;

/// Snapshot of the `Agent` fields that the executor needs.
///
/// This is populated by `Agent::new` and installed into the process
/// global executor slot via `archon_tools::subagent_executor::install_subagent_executor`.
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
    pending_resume_messages: Arc<Mutex<HashMap<String, Vec<serde_json::Value>>>>,
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
    /// Awaitable admission control for foreground/awaited subagent runs.
    ///
    /// `SubagentManager::register_with_id` still enforces the configured
    /// maximum as a defensive invariant, but workflow fanout must experience
    /// the limit as backpressure instead of a hard branch failure.
    subagent_capacity: Arc<Semaphore>,
    /// Session cache for the rendered ARCHON.md hierarchy (#171 Part 5).
    /// Keyed by `working_dir` and revalidated against the discovered files'
    /// `(len, mtime)`, so a 10-agent fan-out reads the hierarchy once.
    archon_md_cache: Arc<crate::archonmd::ArchonMdCache>,
    /// Session cache for rendered `<agent-memory>` blocks (#171 Part 6).
    /// Invalidated from `handle_inner_complete` right after the single
    /// `save_agent_memory` site writes.
    recall_cache: Arc<crate::agents::memory::AgentMemoryRecallCache>,
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
        pending_resume_messages: Arc<Mutex<HashMap<String, Vec<serde_json::Value>>>>,
        agent_config: Arc<crate::agent::AgentConfig>,
        identity: Arc<IdentityProvider>,
    ) -> Self {
        let subagent_capacity =
            Arc::new(Semaphore::new(agent_config.max_subagent_concurrency.max(1)));
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
            subagent_capacity,
            archon_md_cache: Arc::new(crate::archonmd::ArchonMdCache::new()),
            recall_cache: Arc::new(crate::agents::memory::AgentMemoryRecallCache::new()),
        }
    }

    /// Snapshot of the ARCHON.md cache counters (spawn fixtures, #171 Part 5).
    pub fn archon_md_cache_stats(&self) -> crate::archonmd::ArchonMdCacheStats {
        self.archon_md_cache.stats()
    }

    /// Snapshot of the memory-recall cache counters (spawn fixtures, #171 Part 6).
    pub fn recall_cache_stats(&self) -> crate::agents::memory::RecallCacheStats {
        self.recall_cache.stats()
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

        // The board tools are in the default set on purpose. A subagent that
        // finds something outside its assignment has exactly one other place to
        // put it — its return string — which the parent then summarises, and the
        // finding is gone. That is the failure #125 exists to close, so the
        // destination has to be reachable without every caller remembering to
        // ask for it. They are also `Safe`, run-scoped and non-mutating outside
        // the board, so they widen no blast radius the way `Bash` or `Write`
        // (already here) do.
        //
        // This list applies only when neither the request nor the agent
        // definition names any tools. The board tools are additionally in
        // `ALWAYS_ALLOWED` below, which is not subject to that.
        const DEFAULT_TOOLS: &[&str] = &[
            "Read",
            "Grep",
            "Glob",
            "Bash",
            "Write",
            "Edit",
            "BoardRaise",
            "BoardClaim",
            "BoardList",
            "BoardResolve",
        ];

        // How a subagent talks: the board is how it hands work back, and
        // `SendMessage` is how it reaches its lead and its peers. Withholding
        // either does not restrict what an agent can DO — it removes its ability
        // to say what it found. So these are unioned in however `base_allowed`
        // was derived.
        //
        // Without this, an agent spawned with an explicit `allowed_tools` list
        // could not reach the board at all, and most pipeline agents name their
        // tools. The feature was therefore absent from exactly the runs it was
        // built for: fan-outs where several agents share one run.
        //
        // `SendMessage` is here for the same reason and was found the same way.
        // #184 M1 fixed the routing so a subagent's message actually reaches its
        // target, and M5 made team members addressable by role — but no built-in
        // definition names `SendMessage`, so a live two-agent team could not use
        // any of it. The `explore` agent, asked to message a teammate, correctly
        // reported it had no such tool. Routing an agent can never invoke is
        // exactly the #153 shape: the machinery works and nothing reaches it.
        //
        // `DENYLIST` still wins, so this is an always-OFFER set rather than an
        // override of a deliberate refusal.
        const ALWAYS_ALLOWED: &[&str] = &[
            "BoardRaise",
            "BoardClaim",
            "BoardList",
            "BoardResolve",
            "SendMessage",
        ];

        let mut base_allowed: Vec<&str> = if !request.allowed_tools.is_empty() {
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

        for name in ALWAYS_ALLOWED {
            if !DENYLIST.contains(name) && !base_allowed.contains(name) {
                base_allowed.push(name);
            }
        }

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

    fn max_concurrency(&self) -> Option<usize> {
        // Authoritative cap the session `SubagentManager` was constructed with
        // (config.subagent.max_concurrent, threaded via AgentConfig). Fan-out
        // schedulers clamp to this so overflow items wait for a slot instead of
        // being hard-rejected with `SubagentError::MaxConcurrent`.
        Some(self.agent_config.max_subagent_concurrency)
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

    async fn run_to_completion_with_system(
        &self,
        subagent_id: String,
        request: SubagentRequest,
        system: Vec<serde_json::Value>,
        ctx: ToolContext,
        cancel: CancellationToken,
    ) -> Result<String, ExecutorError> {
        self.run_subagent_to_completion_with_system(subagent_id, request, system, ctx, cancel)
            .await
    }

    async fn on_inner_complete(&self, subagent_id: String, result: Result<String, String>) {
        self.handle_inner_complete(subagent_id, result).await;
    }

    /// Trip an agent's shutdown flag, the same one `shutdown_request` sets.
    ///
    /// `TeamDelete` reaches the manager through here, because archon-tools
    /// cannot reach it directly (#184 M5).
    async fn request_shutdown(&self, subagent_id: &str) -> bool {
        self.subagent_manager
            .lock()
            .await
            .request_shutdown(subagent_id)
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

    /// Announce that an agent outlived the auto-background timer.
    ///
    /// Synchronous by contract — see the trait — so the work is spawned rather
    /// than awaited. The timer path returns immediately either way; the lead
    /// picks the envelope up at its next round boundary.
    ///
    /// The manager is the only state this touches, and it is an `Arc`, so the
    /// spawned task holds nothing borrowed from the executor.
    fn on_auto_backgrounded(&self, subagent_id: &str) {
        let manager = Arc::clone(&self.subagent_manager);
        let subagent_id = subagent_id.to_string();

        archon_observability::spawn_named("subagent-idle-notice", async move {
            let name = {
                let mgr = manager.lock().await;
                mgr.get_status(&subagent_id)
                    .and_then(|info| info.request.subagent_type.clone())
            };

            let envelope = archon_tools::send_message::build_agent_status_envelope(
                &subagent_id,
                name.as_deref(),
                archon_tools::send_message::AgentStatusKind::Idle,
                Some("still running after the auto-background timer expired"),
            );

            let mut mgr = manager.lock().await;
            if mgr.pending_message_count(crate::message_router::LEAD_QUEUE_ID)
                >= crate::message_router::MAX_PENDING_MESSAGES
            {
                tracing::warn!(subagent_id, "lead inbox is full; dropping an idle notice");
                return;
            }
            mgr.queue_pending_message(crate::message_router::LEAD_QUEUE_ID, envelope);
        });
    }
}
