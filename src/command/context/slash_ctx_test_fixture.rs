//! Test-only `SlashCommandContext` fixture.
//!
//! Issue #37 AC#2 was "no permission-mode event logs `Cannot find requested
//! stored relation 'permission_runtime_events'` due to DB handle mismatch".
//! The fix lives at `effects.rs`, in the `CommandEffect::SetPermissionMode`
//! arm: the event write must go to `governed_learning_db`, not `cozo_db`.
//!
//! Every earlier test in this area stopped short of that line. The handler
//! tests in `permissions_tests.rs` only prove the effect is *stashed*; the
//! `apply_effect` harness in `tests.rs` marks `SetPermissionMode` as
//! `unreachable!()`; and the one governed-DB assertion
//! (`permissions_bypass_denial_records_to_governed_learning_db`) covers the
//! *denial* path, which writes from inside the handler and never reaches
//! `apply_effect` at all. So the success path — plain `/permissions <mode>`
//! and `/plan` — had no coverage on the handle selection that broke.
//!
//! Closing that gap needs a real `SlashCommandContext`, because the handle
//! selection *is* a field read on that struct: any narrower harness would
//! re-implement the choice under test instead of exercising it. The struct
//! has ~40 fields, which is why `tests.rs` documented a decision not to
//! build one. This module pays that cost once, in a single place, and keeps
//! it honest: `cozo_db` and `governed_learning_db` are deliberately
//! *different* database handles, so reverting the call site to `cozo_db`
//! makes the assertion fail rather than silently pass.
//!
//! Everything the `SetPermissionMode` arm does not read is filled with the
//! cheapest inert value available (`::empty()`, `::default()`, `None`).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use async_trait::async_trait;

use crate::slash_context::SlashCommandContext;

/// LLM stub for `SlashCommandContext::llm_adapter`. No test that uses this
/// fixture runs a pipeline; a call here means the fixture leaked into a code
/// path it was never meant to cover, so it fails loudly.
struct UnusedLlmClient;

#[async_trait]
impl archon_pipeline::runner::LlmClient for UnusedLlmClient {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> anyhow::Result<archon_pipeline::runner::LlmResponse> {
        anyhow::bail!("SlashCommandContext fixture must not reach the LLM adapter")
    }
}

/// Owns the temporary directory backing the fixture's `SessionStore`.
///
/// Held by the test for as long as the context is alive: dropping it removes
/// the session database from disk.
pub struct SlashCtxFixture {
    pub ctx: SlashCommandContext,
    _tempdir: tempfile::TempDir,
}

/// Build a `SlashCommandContext` for `apply_effect` tests.
///
/// `session_id`, `permission_mode`, `cozo_db` and `governed_learning_db` are
/// caller-supplied because they are the inputs the permission-mode effect
/// actually reads. The two database handles are separate parameters on
/// purpose — pass distinct instances to prove which one is written.
pub fn build_test_slash_context(
    session_id: &str,
    initial_permission_mode: &str,
    cozo_db: Option<Arc<cozo::DbInstance>>,
    governed_learning_db: Option<Arc<cozo::DbInstance>>,
) -> SlashCtxFixture {
    let tempdir = tempfile::tempdir().expect("fixture tempdir");
    let session_store = Arc::new(
        archon_session::storage::SessionStore::open(&tempdir.path().join("sessions.db"))
            .expect("fixture session store"),
    );
    let memory: Arc<dyn archon_memory::MemoryTrait> =
        Arc::new(archon_test_support::memory::MockMemoryTrait::new());
    // The TUI channel is not a context field — `apply_effect` takes the
    // sender as its own argument, so the caller owns that pair.
    let registry: Arc<crate::command::registry::Registry> =
        Arc::new(crate::command::registry::default_registry());
    let dispatcher = Arc::new(crate::command::dispatcher::Dispatcher::new(Arc::clone(
        &registry,
    )));

    let ctx = SlashCommandContext {
        fast_mode_shared: Arc::new(AtomicBool::new(false)),
        effort_level_shared: Arc::new(tokio::sync::Mutex::new(
            archon_llm::effort::EffortLevel::Medium,
        )),
        model_override_shared: Arc::new(tokio::sync::Mutex::new(String::new())),
        default_model: "test-model".to_string(),
        context_window: 200_000,
        context_source: "fixture".to_string(),
        show_thinking: Arc::new(AtomicBool::new(false)),
        session_stats: Arc::new(tokio::sync::Mutex::new(
            archon_core::agent::SessionStats::default(),
        )),
        permission_mode: Arc::new(tokio::sync::Mutex::new(initial_permission_mode.to_string())),
        plan_mode_state: Arc::new(tokio::sync::Mutex::new(
            archon_core::agent::plan_mode_state::PlanModeState::default(),
        )),
        session_id: session_id.to_string(),
        session_store,
        cost_config: archon_core::config::CostConfig::default(),
        codex_models: archon_core::config::OpenAiCodexModelsConfig::default(),
        anthropic_models: archon_core::config::AnthropicModelsConfig::default(),
        memory: Arc::clone(&memory),
        garden_config: archon_memory::garden::GardenConfig::default(),
        mcp_manager: archon_mcp::lifecycle::McpServerManager::new(),
        working_dir: tempdir.path().to_path_buf(),
        extra_dirs: Arc::new(tokio::sync::Mutex::new(Vec::new())),
        auth_label: "fixture".to_string(),
        config_path: tempdir.path().join("config.toml"),
        env_vars: archon_core::env_vars::load_env_vars_from(&HashMap::new()),
        config_sources: archon_core::config_source::ConfigSourceMap::default(),
        skill_registry: Arc::new(archon_core::skills::SkillRegistry::new()),
        last_assistant_response: Arc::new(tokio::sync::Mutex::new(String::new())),
        system_prompt_chars: 0,
        tool_defs_chars: 0,
        allow_bypass_permissions: false,
        denial_log: Arc::new(tokio::sync::Mutex::new(
            archon_permissions::denial_log::DenialLog::default(),
        )),
        agent_registry: Arc::new(std::sync::RwLock::new(
            archon_core::agents::AgentRegistry::empty(),
        )),
        task_service: Arc::new(archon_core::tasks::DefaultTaskService::new(
            Arc::new(archon_core::agents::AgentRegistry::empty()),
            16,
        )),
        coding_pipeline: Arc::new(archon_pipeline::coding::facade::CodingFacade::new()),
        research_pipeline: Arc::new(archon_pipeline::research::facade::ResearchFacade::new(
            memory,
            None,
            tempdir.path().to_string_lossy().to_string(),
            None,
        )),
        llm_adapter: Arc::new(UnusedLlmClient),
        leann: None,
        cozo_db,
        governed_learning_db,
        auto_trainer: None,
        registry,
        dispatcher,
        pending_export_shared: Arc::new(std::sync::Mutex::new(None)),
        sandbox_flag: Arc::new(AtomicBool::new(false)),
        hook_registry: None,
        plugin_enable_state: Arc::new(std::sync::RwLock::new(HashMap::new())),
        cancel_handle: Arc::new(std::sync::Mutex::new(None)),
        agent_dispatcher: Arc::new(std::sync::Mutex::new(archon_tui::AgentDispatcher::new(
            Arc::new(crate::agent_handle::NoopAgentRouter),
            tokio::sync::mpsc::channel(1).0,
        ))),
    };

    SlashCtxFixture {
        ctx,
        _tempdir: tempdir,
    }
}
