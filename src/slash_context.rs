//! Slash command context — shared state for all slash command handlers.
//! Extracted from main.rs to enable modular handler extraction.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use archon_core::agent::SessionStats;
use archon_core::skills::SkillRegistry;
use archon_llm::effort::EffortLevel;
use archon_mcp::lifecycle::McpServerManager;
use archon_memory::MemoryTrait;
use archon_memory::garden::GardenConfig;

use crate::command::dispatcher::Dispatcher;
use crate::command::registry::Registry;

/// Groups all shared state needed by slash command handlers so we do not need
/// a dozen individual function parameters.
pub(crate) struct SlashCommandContext {
    pub(crate) fast_mode_shared: Arc<AtomicBool>,
    pub(crate) effort_level_shared: Arc<tokio::sync::Mutex<EffortLevel>>,
    pub(crate) model_override_shared: Arc<tokio::sync::Mutex<String>>,
    pub(crate) default_model: String,
    pub(crate) show_thinking: Arc<AtomicBool>,
    pub(crate) session_stats: Arc<tokio::sync::Mutex<SessionStats>>,
    pub(crate) permission_mode: Arc<tokio::sync::Mutex<String>>,
    pub(crate) session_id: String,
    pub(crate) cost_config: archon_core::config::CostConfig,
    pub(crate) memory: Arc<dyn MemoryTrait>,
    pub(crate) garden_config: GardenConfig,
    pub(crate) mcp_manager: McpServerManager,
    pub(crate) working_dir: PathBuf,
    /// Additional working directories added via `/add-dir`.
    pub(crate) extra_dirs: Arc<tokio::sync::Mutex<Vec<PathBuf>>>,
    pub(crate) auth_label: String,
    pub(crate) config_path: PathBuf,
    pub(crate) env_vars: archon_core::env_vars::ArchonEnvVars,
    pub(crate) config_sources: archon_core::config_source::ConfigSourceMap,
    pub(crate) skill_registry: Arc<SkillRegistry>,
    pub(crate) last_assistant_response: Arc<tokio::sync::Mutex<String>>,
    /// Pre-computed character count of all system prompt blocks (for /context).
    pub(crate) system_prompt_chars: usize,
    /// Pre-computed character count of all tool definition JSON (for /context).
    pub(crate) tool_defs_chars: usize,
    /// Whether `--allow-dangerously-skip-permissions` was passed (unlocks bypassPermissions mode).
    pub(crate) allow_bypass_permissions: bool,
    /// Shared denial log for `/denials` display.
    pub(crate) denial_log: Arc<tokio::sync::Mutex<archon_permissions::denial_log::DenialLog>>,
    /// Agent registry for agent management skills.
    pub(crate) agent_registry: Arc<std::sync::RwLock<archon_core::agents::AgentRegistry>>,
    /// TASK-DS-001: Async TaskService for non-blocking agent/pipeline
    /// invocation from TUI per REQ-ASYNC-001. Constructed once at
    /// session bootstrap; cloned into `CommandContext` per-dispatch
    /// via the DIRECT pattern.
    pub(crate) task_service: Arc<dyn archon_core::tasks::TaskService>,
    /// Coding pipeline facade — constructed once at bootstrap, cloned
    /// into per-dispatch CommandContext via DIRECT pattern. Handlers
    /// call `set_tui_sender()` before `run_pipeline()`.
    pub(crate) coding_pipeline: Arc<archon_pipeline::coding::facade::CodingFacade>,
    /// Research pipeline facade — constructed once at bootstrap.
    pub(crate) research_pipeline: Arc<archon_pipeline::research::facade::ResearchFacade>,
    /// Shared LLM client for pipeline execution.
    pub(crate) llm_adapter: Arc<dyn archon_pipeline::runner::LlmClient>,
    /// LEANN semantic code index for pipeline deep-search context.
    /// Created at bootstrap; None if CozoDB fails to open.
    pub(crate) leann: Option<Arc<archon_pipeline::runner::LeannIntegration>>,
    /// CozoDB instance for learning subsystem persistence (GNN weights,
    /// trajectories, Adam state, training runs). Created at bootstrap.
    pub(crate) cozo_db: Option<Arc<cozo::DbInstance>>,
    /// GNN auto-trainer Arc — Some when the background loop is running.
    /// `/learning-status` reads live state from this; None means the trainer
    /// is disabled in config OR the learning CozoDB failed to open.
    /// Reference: `archon-pipeline/src/learning/gnn/auto_trainer.rs`.
    pub(crate) auto_trainer: Option<Arc<archon_pipeline::learning::gnn::auto_trainer::AutoTrainer>>,
    #[allow(dead_code)]
    pub(crate) registry: Arc<Registry>,
    #[allow(dead_code)]
    pub(crate) dispatcher: Arc<Dispatcher>,
    /// TASK-AGS-POST-6-EXPORT-MIGRATE SIDECAR-SLOT shared slot.
    ///
    /// `/export` moved from the upstream session.rs intercept into
    /// `ExportHandler::execute` via the SIDECAR-SLOT pattern.
    /// EFFECT-SLOT was rejected because `command::context::apply_effect`
    /// runs with only `SlashCommandContext` access and `apply_effect`
    /// is invoked from inside slash.rs (which MUST stay zero-diff for
    /// this ticket). /export needs `agent.lock().await` on the tokio
    /// Mutex-guarded `Agent` that only session.rs has in scope.
    ///
    /// The sync handler parses and validates the format arg, then
    /// writes the parsed `ExportDescriptor` into this shared
    /// `std::sync::Mutex`. After `handle_slash_command` returns
    /// (inside session.rs's `if handled {` branch), the drain takes
    /// the descriptor back out and performs the full mutex-requiring
    /// export I/O where `agent` and `session_id` are in scope.
    ///
    /// `std::sync::Mutex` (not `tokio::sync::Mutex`) because the
    /// handler is sync (no `.await`) and the drain site acquires
    /// briefly only to `.take()` the descriptor — no `.await` held
    /// across any lock. Single-shot per dispatch by construction.
    pub(crate) pending_export_shared:
        Arc<std::sync::Mutex<Option<crate::command::export::ExportDescriptor>>>,
    /// GHOST-006: shared sandbox flag toggled by /sandbox on/off and read by
    /// both tool-execution dispatch paths through the SandboxBackend trait.
    pub(crate) sandbox_flag: Arc<AtomicBool>,
}
