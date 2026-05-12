use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::slash_context::SlashCommandContext;
use archon_core::agent::SessionStats;
use archon_llm::effort::EffortLevel;
use archon_memory::MemoryTrait;

pub(super) struct SlashContextBuildInput {
    pub fast_mode_shared: Arc<AtomicBool>,
    pub effort_level_shared: Arc<tokio::sync::Mutex<EffortLevel>>,
    pub model_override_shared: Arc<tokio::sync::Mutex<String>>,
    pub default_model: String,
    pub show_thinking: Arc<AtomicBool>,
    pub session_stats: Arc<tokio::sync::Mutex<SessionStats>>,
    pub permission_mode: Arc<tokio::sync::Mutex<String>>,
    pub session_id: String,
    pub cost_config: archon_core::config::CostConfig,
    pub memory: Arc<dyn MemoryTrait>,
    pub garden_config: archon_memory::garden::GardenConfig,
    pub mcp_manager: archon_mcp::lifecycle::McpServerManager,
    pub working_dir: std::path::PathBuf,
    pub extra_dirs: Arc<tokio::sync::Mutex<Vec<std::path::PathBuf>>>,
    pub auth_label: String,
    pub config_path: std::path::PathBuf,
    pub env_vars: archon_core::env_vars::ArchonEnvVars,
    pub cli_settings: Option<std::path::PathBuf>,
    pub layer_filter: Option<Vec<archon_core::config_layers::ConfigLayer>>,
    pub last_assistant_response: Arc<tokio::sync::Mutex<String>>,
    pub system_prompt_chars: usize,
    pub tool_defs_chars: usize,
    pub allow_bypass_permissions: bool,
    pub denial_log: Arc<tokio::sync::Mutex<archon_permissions::denial_log::DenialLog>>,
    pub agent_registry: Arc<std::sync::RwLock<archon_core::agents::AgentRegistry>>,
    pub task_service: Arc<dyn archon_core::tasks::TaskService>,
    pub coding_pipeline: Arc<archon_pipeline::coding::facade::CodingFacade>,
    pub research_pipeline: Arc<archon_pipeline::research::facade::ResearchFacade>,
    pub llm_adapter: Arc<dyn archon_pipeline::runner::LlmClient>,
    pub leann: Option<Arc<archon_pipeline::runner::LeannIntegration>>,
    pub sandbox_flag: Arc<AtomicBool>,
    pub hook_registry: Arc<archon_core::hooks::HookRegistry>,
    pub cancel_handle: Arc<std::sync::Mutex<Option<Arc<crate::agent_handle::AgentHandle>>>>,
    pub agent_dispatcher: Arc<std::sync::Mutex<archon_tui::AgentDispatcher>>,
    pub cozo_db: Option<Arc<cozo::DbInstance>>,
    pub auto_trainer:
        Option<Arc<archon_pipeline::learning::gnn::auto_trainer::AutoTrainer>>,
}

pub(super) fn build(input: SlashContextBuildInput) -> SlashCommandContext {
    let registry: Arc<crate::command::registry::Registry> =
        Arc::new(crate::command::registry::default_registry());
    let dispatcher = Arc::new(crate::command::dispatcher::Dispatcher::new(Arc::clone(&registry)));
    SlashCommandContext {
        fast_mode_shared: input.fast_mode_shared,
        effort_level_shared: input.effort_level_shared,
        model_override_shared: input.model_override_shared,
        default_model: input.default_model,
        show_thinking: input.show_thinking,
        session_stats: input.session_stats,
        permission_mode: input.permission_mode,
        session_id: input.session_id,
        cost_config: input.cost_config,
        memory: input.memory,
        garden_config: input.garden_config,
        mcp_manager: input.mcp_manager,
        working_dir: input.working_dir.clone(),
        extra_dirs: input.extra_dirs,
        auth_label: input.auth_label,
        config_path: input.config_path.clone(),
        env_vars: input.env_vars,
        config_sources: archon_core::config_source::ConfigSourceMap::from_layered_load(
            Some(&input.config_path),
            &input.working_dir,
            input.cli_settings.as_deref(),
            input.layer_filter.as_deref(),
        )
        .unwrap_or_default(),
        skill_registry: Arc::new(build_skill_registry(&input.working_dir)),
        last_assistant_response: input.last_assistant_response,
        system_prompt_chars: input.system_prompt_chars,
        tool_defs_chars: input.tool_defs_chars,
        allow_bypass_permissions: input.allow_bypass_permissions,
        denial_log: input.denial_log,
        agent_registry: input.agent_registry,
        task_service: input.task_service,
        coding_pipeline: input.coding_pipeline,
        research_pipeline: input.research_pipeline,
        llm_adapter: input.llm_adapter,
        leann: input.leann,
        registry,
        dispatcher,
        pending_export_shared: Arc::new(std::sync::Mutex::new(None)),
        sandbox_flag: input.sandbox_flag,
        hook_registry: Some(input.hook_registry),
        plugin_enable_state: crate::command::plugin::load_plugin_enable_state(),
        cancel_handle: input.cancel_handle,
        agent_dispatcher: input.agent_dispatcher,
        cozo_db: input.cozo_db,
        auto_trainer: input.auto_trainer,
    }
}

fn build_skill_registry(working_dir: &std::path::Path) -> archon_core::skills::SkillRegistry {
    let mut reg = archon_core::skills::builtin::register_builtins();
    for skill in archon_core::skills::discovery::discover_user_skills(working_dir) {
        tracing::debug!("discovered user skill: {}", skill.name);
        reg.register(Box::new(skill));
    }
    reg.register_alias("?", "help");
    reg
}
