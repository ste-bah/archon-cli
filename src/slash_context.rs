//! Slash command context — shared state for all slash command handlers.
//! Extracted from main.rs to enable modular handler extraction.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use archon_core::agent::SessionStats;
use archon_core::skills::SkillRegistry;
use archon_llm::effort::EffortLevel;
use archon_memory::garden::GardenConfig;
use archon_memory::MemoryTrait;
use archon_mcp::lifecycle::McpServerManager;

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
    #[allow(dead_code)]
    pub(crate) registry: Arc<Registry>,
    #[allow(dead_code)]
    pub(crate) dispatcher: Arc<Dispatcher>,
}
