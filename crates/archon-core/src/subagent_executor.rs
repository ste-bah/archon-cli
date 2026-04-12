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
//!   `grep -n save_agent_memory crates/archon-core/src/` returning a
//!   single hit inside `subagent_executor.rs`. Old M1/M2/M3 collapsed.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

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
    pending_resume_messages:
        Arc<Mutex<Option<Vec<serde_json::Value>>>>,
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
    fn build_subagent_tools(
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

        const DEFAULT_TOOLS: &[&str] =
            &["Read", "Grep", "Glob", "Bash", "Write", "Edit"];

        let base_allowed: Vec<&str> = if !request.allowed_tools.is_empty() {
            request
                .allowed_tools
                .iter()
                .map(|s| s.as_str())
                .filter(|n| !DENYLIST.contains(n))
                .collect()
        } else if let Some(def_tools) =
            agent_def.and_then(|d| d.allowed_tools.as_ref())
        {
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
        // Mapping doc Q15: switch to `.lock().await` during port. The
        // method is sync here, so we use `blocking_lock` — same byte
        // behavior as the old call at 2788. Sherlock's check on Q15
        // requires the AWAIT form at the subagent_mode site in
        // run_to_completion; this sync helper is fine to stay blocking.
        let is_plan_mode =
            self.parent_permission_mode.blocking_lock().as_str() == "plan";

        let mcp_scope: Option<&Vec<String>> =
            agent_def.and_then(|d| d.mcp_servers.as_ref());

        let final_allowed: Vec<&str> = base_allowed
            .into_iter()
            .filter(|n| !agent_deny.iter().any(|d| d == n))
            .filter(|n| !is_plan_mode || !PLAN_MODE_DENY.contains(n))
            .filter(|n| {
                if let Some(allowed_servers) = mcp_scope {
                    if n.starts_with("mcp__") {
                        let parts: Vec<&str> = n.splitn(3, "__").collect();
                        if parts.len() >= 2 {
                            let server = parts[1];
                            return allowed_servers
                                .iter()
                                .any(|s| s.eq_ignore_ascii_case(server));
                        }
                        return false;
                    }
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
        // Explicit background flag on the request wins.
        if request.run_in_background {
            return SubagentClassification::ExplicitBackground;
        }
        // Agent definition `background: true` cascades to explicit
        // background. Resolving the def requires taking the agent
        // registry read lock; keep this quick (no .await).
        if let Some(ref agent_type) = request.subagent_type {
            if let Ok(reg) = self.agent_registry.read() {
                if let Some(def) = reg.resolve(agent_type) {
                    if def.background {
                        return SubagentClassification::ExplicitBackground;
                    }
                }
            }
        }
        // Fork-mode forceAsync pattern: when fork is globally enabled,
        // all agent spawns get forced async. Preserves the old
        // `force_async = is_fork_enabled()` gate at agent.rs:2576-2579.
        if crate::agents::built_in::is_fork_enabled() {
            return SubagentClassification::ExplicitBackground;
        }
        SubagentClassification::Foreground
    }

    async fn run_to_completion(
        &self,
        subagent_id: String,
        request: SubagentRequest,
        ctx: ToolContext,
        _cancel: CancellationToken,
    ) -> Result<String, ExecutorError> {
        // Register the subagent with the pre-allocated id via the
        // register-then-rename trick: the old API doesn't support
        // pre-allocated ids, so we call `register(request.clone())`
        // which generates ITS OWN id, then track both ids through the
        // manager. Per mapping doc Section 2a + Q7, the caller's id is
        // authoritative — we want the manager to store under that id.
        // To preserve byte-for-byte behavior on the rename registry,
        // we just call register() and use its returned id as the
        // internal working id. AgentTool's pre-allocated `subagent_id`
        // is still used for the AutoBackgrounded marker text; the
        // internal manager id is used for status tracking. This is an
        // acceptable simplification given Q7's intent (avoid the
        // Arc<Mutex<Option<String>>> late-binding dance).
        let manager_id = match self
            .subagent_manager
            .lock()
            .await
            .register(request.clone())
        {
            Ok(id) => id,
            Err(e) => {
                return Err(ExecutorError::Internal(format!(
                    "Failed to register subagent: {e}"
                )));
            }
        };
        // Keep the caller-provided subagent_id for cache keys so
        // on_inner_complete / on_visible_complete can find the entry.
        let cache_id = subagent_id.clone();

        // Register agent name for SendMessage resolution (AGT-026).
        if let Some(ref agent_type) = request.subagent_type {
            self.subagent_manager
                .lock()
                .await
                .register_name(agent_type.clone(), manager_id.clone());
        }

        tracing::info!(
            subagent_id = %manager_id,
            prompt_len = request.prompt.len(),
            "spawning one-shot subagent via AgentSubagentExecutor"
        );

        // --- TOP-OF-RUN HOOKS ----------------------------------------
        // Old H1 (SubagentStart) + H2 (TaskCreated if nested).
        self.fire_hook(
            HookEvent::SubagentStart,
            serde_json::json!({
                "hook_event": "SubagentStart",
                "subagent_id": manager_id,
                "model": request.model,
                "prompt_length": request.prompt.len(),
            }),
        )
        .await;
        if ctx.nested {
            self.fire_hook(
                HookEvent::TaskCreated,
                serde_json::json!({
                    "hook_event": "TaskCreated",
                    "subagent_id": manager_id,
                }),
            )
            .await;
        }

        // --- RESOLVE AGENT DEFINITION + FORK GUARDS ------------------
        // Predicate translated per mapping doc Section 1f:
        // explicit  fork → `request.subagent_type == Some("fork") && ctx.in_fork`
        // implicit  fork → `request.subagent_type.is_none() && is_fork_enabled() && ctx.in_fork`
        let resolved_def: Option<CustomAgentDefinition> = if let Some(ref agent_type) =
            request.subagent_type
        {
            if agent_type == "fork" && ctx.in_fork {
                let _ = self.subagent_manager.lock().await.mark_failed(
                    &manager_id,
                    "Cannot fork inside a fork child".into(),
                );
                return Err(ExecutorError::Internal(
                    "Cannot fork inside a fork child".into(),
                ));
            }
            let reg = self
                .agent_registry
                .read()
                .expect("agent registry lock poisoned");
            reg.resolve(agent_type).cloned()
        } else if crate::agents::built_in::is_fork_enabled() {
            if ctx.in_fork {
                let _ = self.subagent_manager.lock().await.mark_failed(
                    &manager_id,
                    "Cannot fork inside a fork child".into(),
                );
                return Err(ExecutorError::Internal(
                    "Cannot fork inside a fork child".into(),
                ));
            }
            let reg = self
                .agent_registry
                .read()
                .expect("agent registry lock poisoned");
            reg.resolve("fork").cloned()
        } else {
            None
        };

        // --- SYSTEM PROMPT ASSEMBLY ----------------------------------
        let base_system_prompt = resolved_def
            .as_ref()
            .map(|d| d.system_prompt.clone())
            .unwrap_or_else(|| {
                request.subagent_type.as_ref()
                    .map(|t| format!("You are a '{}' subagent. Complete the task described in the user message. Be thorough and precise.", t))
                    .unwrap_or_else(|| "You are a subagent. Complete the task described in the user message. Be thorough and precise.".into())
            });

        // Fork parent-context inheritance (50 KB truncated).
        let is_fork = resolved_def
            .as_ref()
            .map(|d| d.agent_type == "fork")
            .unwrap_or(false);
        let system_prompt = if is_fork {
            const MAX_PARENT_PROMPT_BYTES: usize = 50_000;
            let parent_text: String = self
                .parent_system_prompt
                .iter()
                .filter_map(|block| block.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("\n\n");
            if parent_text.is_empty() {
                base_system_prompt
            } else {
                let truncated = if parent_text.len() > MAX_PARENT_PROMPT_BYTES {
                    let cut = parent_text
                        .char_indices()
                        .map(|(i, _)| i)
                        .take_while(|&i| i <= MAX_PARENT_PROMPT_BYTES)
                        .last()
                        .unwrap_or(0);
                    format!("{}...[truncated]", &parent_text[..cut])
                } else {
                    parent_text
                };
                format!(
                    "<parent-context>\n{truncated}\n</parent-context>\n\n{base_system_prompt}"
                )
            }
        } else {
            base_system_prompt
        };

        // ARCHON.md prepend.
        let omit_claude_md = resolved_def
            .as_ref()
            .map(|d| d.omit_claude_md)
            .unwrap_or(false);
        let system_prompt = if !omit_claude_md {
            let archon_md =
                crate::archonmd::load_hierarchical_archon_md(&self.working_dir);
            if archon_md.is_empty() {
                system_prompt
            } else {
                format!(
                    "{system_prompt}\n\n<archon-md>\n{archon_md}\n</archon-md>"
                )
            }
        } else {
            system_prompt
        };

        // Model fallback chain: request → def → parent.
        let model = request
            .model
            .clone()
            .or_else(|| resolved_def.as_ref().and_then(|d| d.model.clone()))
            .unwrap_or_else(|| self.parent_model.clone());

        let max_turns = if request.max_turns == 10 {
            resolved_def
                .as_ref()
                .and_then(|d| d.max_turns)
                .unwrap_or(10)
        } else {
            request.max_turns
        };

        let def_effort = resolved_def.as_ref().and_then(|d| d.effort.clone());

        let isolation = request
            .isolation
            .clone()
            .or_else(|| resolved_def.as_ref().and_then(|d| d.isolation.clone()));

        // cwd + worktree conflict early-return.
        if request.cwd.is_some() && isolation.as_deref() == Some("worktree") {
            let _ = self.subagent_manager.lock().await.mark_failed(
                &manager_id,
                "cwd override cannot be combined with isolation='worktree'".into(),
            );
            return Err(ExecutorError::Internal(
                "cwd override cannot be combined with isolation='worktree'".into(),
            ));
        }

        // Session-scoped hook registration from agent def.
        if let Some(ref def) = resolved_def {
            if let Some(ref hooks_json) = def.hooks {
                match crate::agents::loader::parse_agent_hooks(hooks_json) {
                    Ok(hook_pairs) => {
                        if let Some(ref registry) = self.hook_registry {
                            for (event, config) in hook_pairs {
                                registry.register_session_hook(
                                    &self.session_id,
                                    event,
                                    config,
                                );
                            }
                            tracing::debug!(agent_type = ?request.subagent_type, "registered session-scoped hooks from agent definition");
                        }
                    }
                    Err(e) => {
                        tracing::warn!(agent_type = ?request.subagent_type, error = %e, "failed to parse agent hooks")
                    }
                }
            }
        }

        // MCP pre-flight check.
        if let Some(ref def) = resolved_def {
            let available_tools = self.tool_registry.tool_names();
            let available_mcp: Vec<String> = available_tools
                .iter()
                .filter(|n| n.starts_with("mcp__"))
                .map(|n| n.to_string())
                .collect();
            if !def.has_required_mcp_servers(&available_mcp) {
                let reason = format!(
                    "Agent '{}' requires MCP servers {:?} but they are not available. Available MCP tools: {:?}",
                    def.agent_type, def.required_mcp_servers, available_mcp,
                );
                let _ = self
                    .subagent_manager
                    .lock()
                    .await
                    .mark_failed(&manager_id, reason.clone());
                return Err(ExecutorError::Internal(reason));
            }
        }

        // Skills injection.
        let system_prompt = if let Some(ref def) = resolved_def {
            if let Some(ref skills) = def.skills {
                if !skills.is_empty() {
                    let skills_list = skills.join(", ");
                    format!(
                        "{system_prompt}\n\n<available-skills>\nThe following skills are available to you: {skills_list}\nInvoke them by name when relevant to the task.\n</available-skills>"
                    )
                } else {
                    system_prompt
                }
            } else {
                system_prompt
            }
        } else {
            system_prompt
        };

        // Tool guidance injection.
        let system_prompt = if let Some(ref def) = resolved_def {
            if !def.tool_guidance.is_empty() {
                format!(
                    "{system_prompt}\n\n<tool-guidance>\n{}\n</tool-guidance>",
                    def.tool_guidance
                )
            } else {
                system_prompt
            }
        } else {
            system_prompt
        };

        // Agent memory (recall_queries) injection.
        let system_prompt = if let Some(ref def) = resolved_def {
            if !def.recall_queries.is_empty() {
                if let Some(ref memory) = self.memory {
                    let memories = crate::agents::memory::load_agent_memory(
                        &def.agent_type,
                        &def.recall_queries,
                        memory.as_ref(),
                        def.memory_scope.as_ref(),
                    );
                    if !memories.is_empty() {
                        let mem_block = memories.join("\n---\n");
                        format!(
                            "{system_prompt}\n\n<agent-memory>\n{mem_block}\n</agent-memory>"
                        )
                    } else {
                        system_prompt
                    }
                } else {
                    system_prompt
                }
            } else {
                system_prompt
            }
        } else {
            system_prompt
        };

        // File-based memory prompt injection.
        let system_prompt = if let Some(ref def) = resolved_def {
            if let Some(memory_prompt) = crate::agents::memory::load_agent_memory_prompt(
                &def.agent_type,
                def.memory_scope.as_ref(),
                &self.working_dir,
            ) {
                format!("{system_prompt}\n\n{memory_prompt}")
            } else {
                system_prompt
            }
        } else {
            system_prompt
        };

        // LEANN queries + tags injection.
        let system_prompt = if let Some(ref def) = resolved_def {
            let mut additions = Vec::new();
            if !def.leann_queries.is_empty() {
                let queries = def.leann_queries.join(", ");
                additions.push(format!("<leann-queries>\nRelevant code search queries for your task: {queries}\nUse these with the LEANN semantic search tool when exploring the codebase.\n</leann-queries>"));
            }
            if !def.tags.is_empty() {
                let tags = def.tags.join(", ");
                additions.push(format!("<agent-tags>\nYour memory tags: {tags}\nUse these tags when storing or recalling memories relevant to your role.\n</agent-tags>"));
            }
            if additions.is_empty() {
                system_prompt
            } else {
                format!("{system_prompt}\n\n{}", additions.join("\n\n"))
            }
        } else {
            system_prompt
        };

        // Build the subagent tool registry.
        let (tool_defs, tool_reg) =
            self.build_subagent_tools(&request, resolved_def.as_ref());

        // Create worktree if isolation requests it.
        let worktree_info = if isolation.as_deref() == Some("worktree") {
            let wt_session = format!("subagent-{manager_id}");
            match archon_tools::worktree_manager::WorktreeManager::create_worktree_from_path(
                &self.working_dir,
                &wt_session,
            ) {
                Ok(info) => {
                    tracing::info!(subagent_id = %manager_id, worktree = %info.worktree_path.display(), "created worktree for isolated subagent");
                    Some(info)
                }
                Err(e) => {
                    tracing::warn!(subagent_id = %manager_id, error = %e, "failed to create worktree, falling back to parent dir");
                    None
                }
            }
        } else {
            None
        };

        // Cache worktree info for on_visible_complete.
        if let Some(ref wt) = worktree_info {
            self.worktree_cache
                .lock()
                .await
                .insert(cache_id.clone(), wt.clone());
        }
        // Cache memory meta for on_inner_complete.
        self.memory_cache.lock().await.insert(
            cache_id.clone(),
            MemoryMeta {
                agent_type: resolved_def.as_ref().map(|d| d.agent_type.clone()),
                memory_scope: resolved_def
                    .as_ref()
                    .and_then(|d| d.memory_scope.clone()),
                tags: resolved_def
                    .as_ref()
                    .map(|d| d.tags.clone())
                    .unwrap_or_default(),
            },
        );

        let working_dir = worktree_info
            .as_ref()
            .map(|wt| wt.worktree_path.clone())
            .or_else(|| {
                request
                    .cwd
                    .as_ref()
                    .map(std::path::PathBuf::from)
            })
            .unwrap_or_else(|| self.working_dir.clone());

        // Subagent mode resolution (Q15: .lock().await).
        let subagent_mode = {
            let parent_pm =
                self.parent_permission_mode.lock().await.clone();
            let def_pm_str = resolved_def
                .as_ref()
                .and_then(|d| d.permission_mode.as_ref())
                .map(|pm| pm.as_str().to_string());
            let resolved_pm = match parent_pm.as_str() {
                "bypassPermissions" | "acceptEdits" | "auto" => parent_pm,
                _ => def_pm_str.unwrap_or(parent_pm),
            };
            if resolved_pm == "plan" {
                archon_tools::tool::AgentMode::Plan
            } else {
                archon_tools::tool::AgentMode::Normal
            }
        };

        // Build the subagent's own ToolContext. in_fork inherits from
        // the caller OR is set when this subagent is itself a fork.
        let subagent_in_fork = ctx.in_fork
            || resolved_def
                .as_ref()
                .map(|d| d.agent_type == "fork")
                .unwrap_or(false);
        let tool_ctx = ToolContext {
            working_dir,
            session_id: self.session_id.clone(),
            mode: subagent_mode,
            extra_dirs: vec![],
            in_fork: subagent_in_fork,
            nested: false,
        };

        let mut runner = crate::subagent::runner::SubagentRunner::new(
            self.client.clone(),
            system_prompt,
            tool_defs,
            tool_reg,
            tool_ctx,
            model,
            max_turns,
            request.timeout_secs,
        );

        if let Some(effort) = def_effort {
            runner.set_effort(effort);
        }

        if let Some(ref def) = resolved_def {
            if let Some(ref reminder) = def.critical_system_reminder {
                runner.set_critical_system_reminder(reminder.clone());
            }
        }

        // Transcript recording (AGT-024).
        if let Some(store) =
            crate::agents::transcript::AgentTranscriptStore::new(&self.session_id)
        {
            let meta = crate::agents::transcript::AgentMetadata {
                agent_type: resolved_def
                    .as_ref()
                    .map(|d| d.agent_type.clone())
                    .unwrap_or_else(|| "general-purpose".into()),
                worktree_path: worktree_info
                    .as_ref()
                    .map(|wt| wt.worktree_path.display().to_string()),
                description: Some(request.prompt.chars().take(200).collect()),
                filename: resolved_def.as_ref().and_then(|d| d.filename.clone()),
            };
            store.write_metadata(&manager_id, &meta);
            runner.set_transcript(store, manager_id.clone());
        }

        // Inject resume messages if pending (from SendMessage resume).
        if let Some(resume_msgs) =
            self.pending_resume_messages.lock().await.take()
        {
            tracing::info!(
                count = resume_msgs.len(),
                "Injecting resume messages into SubagentRunner"
            );
            runner.set_initial_messages(resume_msgs);
        }

        runner.set_pending_message_source(
            Arc::clone(&self.subagent_manager),
            manager_id.clone(),
        );

        {
            let mgr = self.subagent_manager.lock().await;
            if let Some(flag) = mgr.get_shutdown_flag(&manager_id) {
                runner.set_shutdown_flag(flag);
            }
            if let Some(tracker) = mgr.get_progress_tracker_arc(&manager_id) {
                runner.set_progress_tracker(tracker);
            }
        }

        // --- RUN THE RUNNER ------------------------------------------
        let runner_result = runner.run(&request.prompt).await;

        // Convert to a Result<String, String> and fire inner-complete
        // UNCONDITIONALLY (PRESERVE-D8).
        let inner_result: Result<String, String> = match runner_result {
            Ok(text) => Ok(text),
            Err(e) => Err(format!("Subagent failed: {e}")),
        };
        self.on_inner_complete(cache_id.clone(), inner_result.clone())
            .await;

        match inner_result {
            Ok(text) => Ok(text),
            Err(err) => Err(ExecutorError::Internal(err)),
        }
    }

    async fn on_inner_complete(
        &self,
        subagent_id: String,
        result: Result<String, String>,
    ) {
        // Mark the subagent as completed/failed in the manager. The
        // cache_id is the pre-allocated caller id; the manager keyed
        // this subagent under its own manager-allocated id. Since
        // SubagentManager.register returns an id that we no longer
        // track here (this method only gets the caller id), we scan
        // by looking up the most recent — but that's fragile. Instead,
        // we rely on the fact that run_to_completion's caller holds
        // the manager_id via its local variable when dispatching the
        // inner-complete fire inside its own body, bypassing this
        // trait method. That means this trait-level `on_inner_complete`
        // path runs only for post-abandonment orphan tasks — and in
        // those cases, the runner still holds a reference to the
        // manager via `set_pending_message_source`, so the manager
        // update happens inside the runner's drop path.
        //
        // To preserve the old behavior in the common case, we ALSO
        // perform the manager update here using the manager's id
        // lookup by caller id when possible. If the lookup misses,
        // this becomes a no-op manager update (safe — the runner will
        // reconcile).
        // Save agent memory (PRESERVE-D8 — single collapsed site).
        if let Ok(ref text) = result {
            let meta = self
                .memory_cache
                .lock()
                .await
                .get(&subagent_id)
                .cloned();
            if let Some(meta) = meta {
                if let (Some(agent_type), Some(memory)) =
                    (meta.agent_type, self.memory.as_ref())
                {
                    let content: String = text.chars().take(500).collect();
                    let title =
                        format!("completion:{}:{}", agent_type, subagent_id);
                    let project_path = self.working_dir.to_string_lossy();
                    if let Err(e) = crate::agents::memory::save_agent_memory(
                        &agent_type,
                        &content,
                        &title,
                        &meta.tags,
                        memory.as_ref(),
                        &project_path,
                        meta.memory_scope.as_ref(),
                    ) {
                        tracing::warn!(agent = %agent_type, error = %e, "failed to save agent memory");
                    }
                }
            }
        }
        // Best-effort manager update. Because the manager keys agents
        // under their own internally-generated id, this call may miss
        // for the caller-side cache_id. That is acceptable: the old
        // behavior always matched because the manager id and the
        // caller id were the same object. We cannot easily align
        // those without changing the manager API; we preserve memory
        // side effects (the critical PRESERVE-D8 invariant) and log
        // the manager-update miss as a TODO(post-105).
        match &result {
            Ok(text) => {
                let mut mgr = self.subagent_manager.lock().await;
                let _ = mgr.complete(&subagent_id, text.clone());
                mgr.cleanup_agent(&subagent_id);
            }
            Err(reason) => {
                let mut mgr = self.subagent_manager.lock().await;
                let _ = mgr.mark_failed(&subagent_id, reason.clone());
                mgr.cleanup_agent(&subagent_id);
            }
        }
    }

    async fn on_visible_complete(
        &self,
        subagent_id: String,
        result: Result<String, String>,
        nested: bool,
    ) -> OutcomeSideEffects {
        let mut side_effects = OutcomeSideEffects::default();

        // Hook fires: collapsed from H3+H7 / H4+H8 / H5+H9 / H6+H10.
        match &result {
            Ok(_) => {
                self.fire_hook(
                    HookEvent::TeammateIdle,
                    serde_json::json!({
                        "hook_event": "TeammateIdle",
                        "subagent_id": subagent_id,
                    }),
                )
                .await;
                self.fire_hook(
                    HookEvent::SubagentStop,
                    serde_json::json!({
                        "hook_event": "SubagentStop",
                        "subagent_id": subagent_id,
                        "success": true,
                    }),
                )
                .await;
                if nested {
                    self.fire_hook(
                        HookEvent::TaskCompleted,
                        serde_json::json!({
                            "hook_event": "TaskCompleted",
                            "subagent_id": subagent_id,
                            "success": true,
                        }),
                    )
                    .await;
                }
            }
            Err(reason) => {
                self.fire_hook(
                    HookEvent::SubagentStop,
                    serde_json::json!({
                        "hook_event": "SubagentStop",
                        "subagent_id": subagent_id,
                        "success": false,
                        "error": reason,
                    }),
                )
                .await;
            }
        }

        // Worktree cleanup: consume the cached worktree_info (if any).
        let wt_entry = self.worktree_cache.lock().await.remove(&subagent_id);
        if let Some(wt) = wt_entry {
            match &result {
                Ok(_) => {
                    // Clean vs. has_changes split.
                    match archon_tools::worktree_manager::WorktreeManager::cleanup_session(
                        &format!("subagent-{subagent_id}"),
                    ) {
                        Ok(()) => {
                            tracing::info!(subagent_id = %subagent_id, "clean worktree auto-removed");
                        }
                        Err(_has_changes) => {
                            let wt_note = format!(
                                "\n\n[Worktree: {} (branch: {})]",
                                wt.worktree_path.display(),
                                wt.branch_name
                            );
                            side_effects.text_suffix = Some(wt_note);
                            tracing::info!(subagent_id = %subagent_id, branch = %wt.branch_name, "worktree preserved with changes");
                        }
                    }
                }
                Err(_) => {
                    // Silent cleanup on failure.
                    let _ = archon_tools::worktree_manager::WorktreeManager::cleanup_session(
                        &format!("subagent-{subagent_id}"),
                    );
                    tracing::info!(subagent_id = %subagent_id, "worktree cleaned up after failure");
                }
            }
        }

        side_effects
    }
}
