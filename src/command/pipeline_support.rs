use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use archon_core::agent::AgentConfig;
use archon_core::agents::AgentRegistry;
use archon_core::config::ArchonConfig;
use archon_core::dispatch::create_default_registry;
use archon_core::env_vars::ArchonEnvVars;
use archon_core::subagent::SubagentManager;
use archon_core::subagent_executor::AgentSubagentExecutor;
use archon_llm::identity::{IdentityMode, IdentityProvider};
use archon_llm::provider::LlmProvider;
use archon_pipeline::learning::causal::CausalMemory;
use archon_pipeline::learning::desc::DescEpisodeStore;
use archon_pipeline::learning::integration::{LearningIntegration, LearningIntegrationConfig};
use archon_pipeline::learning::patterns::PatternStore;
use archon_pipeline::learning::reasoning::{ReasoningBank, ReasoningBankConfig, ReasoningBankDeps};
use archon_pipeline::learning::reflexion::ReflexionInjector;
use archon_pipeline::runner::LlmClient;
use archon_tools::tool::ToolContext;

use crate::runtime::llm::build_configured_llm_provider;

// Run-reporting moved to `pipeline_support_result` to keep this file under the
// 500-line ceiling. Re-exported so callers keep importing it from here — the
// split is about file size, not about changing who owns the entry points.
pub(crate) use super::pipeline_support_result::{
    final_research_artifact_paths, print_pipeline_result,
};

pub(crate) async fn build_pipeline_adapter(
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
    origin: &str,
) -> Result<archon_pipeline::llm_adapter::ProviderLlmAdapter> {
    let provider = build_configured_llm_provider(config, env_vars, origin).await?;
    Ok(archon_pipeline::llm_adapter::ProviderLlmAdapter::new(provider).with_origin(origin))
}

pub(crate) async fn build_subagent_pipeline_adapter(
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
    origin: &str,
    cwd: &Path,
    session_id: &str,
) -> Result<Arc<dyn LlmClient>> {
    let provider = build_configured_llm_provider(config, env_vars, origin).await?;
    let raw: Arc<dyn LlmClient> = Arc::new(
        archon_pipeline::llm_adapter::ProviderLlmAdapter::new(Arc::clone(&provider))
            .with_origin(origin),
    );
    let agent_config = workflow_cli_agent_config(config, cwd, session_id);
    install_workflow_cli_subagent_executor(
        config,
        Arc::clone(&provider),
        cwd,
        session_id,
        agent_config.clone(),
    )
    .await;
    let mut tool_context = ToolContext {
        working_dir: cwd.to_path_buf(),
        session_id: session_id.to_string(),
        cancel_parent: agent_config.cancel_token.clone(),
        sandbox: agent_config.sandbox.clone(),
        activity_sink: agent_config.activity_sink.clone(),
        ..ToolContext::default()
    };
    crate::command::world_model::configure_tool_run_context(config, &mut tool_context);
    Ok(Arc::new(
        archon_pipeline::subagent_adapter::SubagentPipelineClient::with_provider(
            raw,
            tool_context,
            provider,
        ),
    ))
}

/// Build the `AgentConfig` workflow subagents run under.
///
/// Every field here must come from `config`, not from `AgentConfig::default()`.
/// The default is Anthropic-shaped — `model: "claude-sonnet-4-6"` — so a
/// workflow on a Codex provider asked it for a model Codex cannot serve and got
/// that provider's own fallback instead. The session path never had this
/// problem because it calls `active_session_model`, whose whole purpose is
/// stated by its test: `..._uses_configured_codex_default_when_claude_default_would_leak`.
/// Measured before the fix: 698 of 704 subagent requests ran on the fallback
/// while `[models.openai-codex]` said otherwise, and editing that config
/// changed nothing because it was never read on this path.
///
/// `max_tokens`/`thinking_budget` and the permission rules were silently
/// defaulted for the same reason: a struct-update from `default()` looks
/// complete at the call site while quietly supplying values the operator never
/// chose. `install_workflow_cli_subagent_executor` extends `permission_rules`
/// with project MCP grants, so seeding it from config here is additive.
fn workflow_cli_agent_config(config: &ArchonConfig, cwd: &Path, session_id: &str) -> AgentConfig {
    AgentConfig {
        model: crate::session::active_session_model(config),
        max_tokens: config.api.resolved_max_tokens(),
        thinking_budget: config.api.thinking_budget,
        permission_rules: archon_permissions::rules::RuleSet {
            always_allow: config.permissions.always_allow.clone(),
            always_deny: config.permissions.always_deny.clone(),
            always_ask: config.permissions.always_ask.clone(),
        },
        working_dir: cwd.to_path_buf(),
        session_id: session_id.to_string(),
        max_tool_concurrency: config.tools.max_concurrency as usize,
        max_subagent_concurrency: config.subagent.max_concurrent.max(1),
        subagent_auto_isolation: config.subagent.auto_isolation,
        subagent_isolation_max_tier: config.subagent.isolation_max_tier,
        context: config.context.clone(),
        ..AgentConfig::default()
    }
}

async fn install_workflow_cli_subagent_executor(
    config: &ArchonConfig,
    provider: Arc<dyn LlmProvider>,
    cwd: &Path,
    session_id: &str,
    mut agent_config: AgentConfig,
) {
    let mut registry = create_default_registry(cwd.to_path_buf(), None);
    registry.replace(Box::new(config.tools.bash_tool(&config.permissions)));
    // #189 Phase 6: the same command lists Bash uses, so typing a command into
    // a persistent shell is gated exactly as hard as running it through Bash.
    registry.replace(Box::new(
        archon_core::config::ToolsConfig::terminal_write_tool(&config.permissions),
    ));
    crate::command::workflow_mcp::install_project_tools(
        cwd,
        &mut registry,
        &mut agent_config.permission_rules,
    )
    .await;
    let subagent_manager = Arc::new(tokio::sync::Mutex::new(SubagentManager::new(
        agent_config.max_subagent_concurrency,
    )));
    let agent_registry = Arc::new(std::sync::RwLock::new(AgentRegistry::load(cwd)));
    let identity = Arc::new(IdentityProvider::new(
        IdentityMode::Clean,
        session_id.to_string(),
        String::new(),
        String::new(),
    ));
    let executor = AgentSubagentExecutor::new(
        provider,
        registry,
        subagent_manager,
        agent_registry,
        None,
        None,
        cwd.to_path_buf(),
        session_id.to_string(),
        agent_config.model.clone(),
        agent_config.system_prompt.clone(),
        Arc::clone(&agent_config.permission_mode),
        Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
        Arc::new(agent_config),
        identity,
    );
    archon_tools::subagent_executor::install_subagent_executor(Arc::new(executor));
}

pub(crate) async fn init_leann(cwd: &Path) -> Option<archon_pipeline::runner::LeannIntegration> {
    let db_path = cwd.join(".archon").join("leann.db");
    if let Some(parent) = db_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match archon_leann::CodeIndex::new(&db_path, Default::default()) {
        Ok(idx) => {
            let li = archon_pipeline::runner::LeannIntegration::new(Arc::new(idx));
            if let Err(e) = li.init_repository(cwd).await {
                tracing::warn!(error = %e, "LEANN init failed; continuing without code context");
            }
            Some(li)
        }
        Err(e) => {
            tracing::warn!(error = %e, "LEANN unavailable; continuing without code context");
            None
        }
    }
}

pub(crate) fn build_interactive_learning_stack(
    config: &ArchonConfig,
    db: Option<Arc<cozo::DbInstance>>,
    auto_trainer: Option<Arc<archon_pipeline::learning::gnn::auto_trainer::AutoTrainer>>,
) -> Option<LearningIntegration> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf());
    let policy = load_learning_policy(&cwd);
    build_learning_stack_from_db(
        config,
        db,
        auto_trainer,
        config.learning.sona.enabled,
        policy.as_ref(),
    )
}

pub(crate) fn build_reflexion_injector(config: &ArchonConfig) -> Option<ReflexionInjector> {
    config
        .learning
        .reflexion
        .enabled
        .then(|| ReflexionInjector::new(config.learning.reflexion.max_per_agent))
}

fn build_learning_stack_from_db(
    config: &ArchonConfig,
    db: Option<Arc<cozo::DbInstance>>,
    auto_trainer: Option<Arc<archon_pipeline::learning::gnn::auto_trainer::AutoTrainer>>,
    track_trajectories: bool,
    learning_policy: Option<&archon_policy::LearningPolicy>,
) -> Option<LearningIntegration> {
    let has_learning = track_trajectories
        || config.learning.reasoning_bank.enabled
        || config.learning.desc.enabled
        || auto_trainer.is_some();
    if !has_learning {
        return None;
    }

    let mut integration_config = LearningIntegrationConfig {
        track_trajectories,
        ..LearningIntegrationConfig::default()
    };
    apply_autonomous_learning_policy(&mut integration_config, learning_policy);
    let mut learning = if let Some(db) = db.clone() {
        LearningIntegration::new_with_persistent_sona(
            db,
            integration_config,
            auto_trainer.clone(),
            config.learning.gnn.input_dim,
        )
    } else {
        integration_config.track_trajectories = false;
        LearningIntegration::new(None, None, integration_config, auto_trainer.clone())
    };

    if config.learning.reasoning_bank.enabled {
        learning = learning.with_reasoning_bank(build_reasoning_bank(config));
    }
    if config.learning.desc.enabled
        && let Some(db) = db
    {
        learning = learning.with_desc_store(DescEpisodeStore::from_arc(db));
    }

    Some(learning)
}

fn build_reasoning_bank(config: &ArchonConfig) -> ReasoningBank {
    let causal_memory = config
        .learning
        .causal_memory
        .enabled
        .then(CausalMemory::new);
    ReasoningBank::new(ReasoningBankDeps {
        pattern_store: PatternStore::new(),
        causal_memory,
        gnn_enhancer: None,
        sona_engine: None,
        config: ReasoningBankConfig::default(),
    })
}

pub(crate) fn build_pipeline_learning_stack(
    config: &ArchonConfig,
    cwd: &Path,
) -> (
    Option<LearningIntegration>,
    Option<Arc<archon_pipeline::learning::gnn::auto_trainer::AutoTrainer>>,
) {
    let db = open_pipeline_learning_db(cwd);
    let auto_trainer = db
        .as_ref()
        .and_then(|db| build_pipeline_auto_trainer_from_db(config, Arc::clone(db)));
    let policy = load_learning_policy(cwd);
    let integration_config = LearningIntegrationConfig {
        track_trajectories: config.learning.sona.enabled && config.learning.sona.pipeline_recording,
        ..LearningIntegrationConfig::default()
    };
    if !integration_config.track_trajectories {
        tracing::info!(
            sona_enabled = config.learning.sona.enabled,
            pipeline_recording = config.learning.sona.pipeline_recording,
            "pipeline SONA trajectory recording disabled"
        );
    }

    let learning = build_learning_stack_from_db(
        config,
        db,
        auto_trainer.clone(),
        integration_config.track_trajectories,
        policy.as_ref(),
    );
    (learning, auto_trainer)
}

fn load_learning_policy(cwd: &Path) -> Option<archon_policy::LearningPolicy> {
    archon_policy::load_effective_policy(cwd)
        .map(|policy| policy.learning)
        .map_err(|e| {
            tracing::warn!(error = %e, "learning policy unavailable; autonomous apply disabled");
            e
        })
        .ok()
}

fn apply_autonomous_learning_policy(
    config: &mut LearningIntegrationConfig,
    policy: Option<&archon_policy::LearningPolicy>,
) {
    let Some(policy) = policy else {
        return;
    };
    config.autonomous_behaviour_apply = policy.autonomous_apply;
    config.autonomous_max_risk =
        archon_learning::models::RiskLevel::from_str(&policy.autonomous_max_risk)
            .unwrap_or(archon_learning::models::RiskLevel::Low);
    config.autonomous_min_evidence = policy.autonomous_min_evidence;
    config.autonomous_max_recent_incidents = policy.autonomous_max_recent_incidents;
}

fn open_pipeline_learning_db(cwd: &Path) -> Option<Arc<cozo::DbInstance>> {
    let db_path = crate::command::store_paths::learning_db_path_for_dir(cwd);
    open_pipeline_learning_db_at(cwd, &db_path)
}

fn open_pipeline_learning_db_at(cwd: &Path, db_path: &Path) -> Option<Arc<cozo::DbInstance>> {
    if let Some(parent) = db_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let db = match archon_learning::cozo_guard::open_sqlite_guarded(
        db_path.to_str().unwrap_or(""),
        "open pipeline learning db",
    ) {
        Ok(db) => db,
        Err(e) => {
            tracing::warn!(error = %e, "pipeline: learning DB unavailable");
            return None;
        }
    };
    if let Err(e) = archon_pipeline::learning::schema::initialize_learning_schemas(&db) {
        tracing::warn!(error = %e, "pipeline: learning schema init failed");
        return None;
    }
    crate::command::pipeline_learning_migration::maybe_migrate_legacy_pipeline_learning_with_log(
        cwd, db_path, &db, "pipeline",
    );
    Some(db)
}

fn build_pipeline_auto_trainer_from_db(
    config: &ArchonConfig,
    db: Arc<cozo::DbInstance>,
) -> Option<Arc<archon_pipeline::learning::gnn::auto_trainer::AutoTrainer>> {
    let at_cfg = &config.learning.gnn.auto_trainer;
    if !at_cfg.enabled || !config.learning.gnn.enabled {
        return None;
    }

    let gnn_cfg = &config.learning.gnn;
    let train_cfg = &gnn_cfg.training;
    let params = archon_pipeline::learning::gnn::auto_trainer_runtime::AutoTrainerBuildParams {
        at_config: archon_pipeline::learning::gnn::auto_trainer::AutoTrainerConfig {
            enabled: at_cfg.enabled,
            min_throttle_ms: at_cfg.min_throttle_ms,
            trigger_new_memories: at_cfg.trigger_new_memories,
            trigger_elapsed_ms: at_cfg.trigger_elapsed_ms,
            trigger_corrections: at_cfg.trigger_corrections,
            first_run_threshold: at_cfg.first_run_threshold,
            max_runtime_ms: at_cfg.max_runtime_ms,
            tick_interval_ms: at_cfg.tick_interval_ms,
        },
        initial_total_memories: 0,
        initial_total_corrections: 0,
        training_config: archon_pipeline::learning::gnn::trainer::TrainingConfig {
            learning_rate: train_cfg.learning_rate,
            batch_size: train_cfg.batch_size,
            max_epochs: train_cfg.max_epochs,
            early_stopping_patience: train_cfg.early_stopping_patience,
            validation_split: train_cfg.validation_split,
            ewc_lambda: train_cfg.ewc_lambda,
            margin: train_cfg.margin,
            triplet_loss_coefficient: train_cfg.triplet_loss_coefficient,
            max_gradient_norm: train_cfg.max_gradient_norm,
            max_triplets_per_run: train_cfg.max_triplets_per_run,
            max_runtime_ms: train_cfg.max_runtime_ms,
            ..Default::default()
        },
        gnn_input_dim: gnn_cfg.input_dim,
        gnn_output_dim: gnn_cfg.output_dim,
        gnn_num_layers: gnn_cfg.num_layers,
        gnn_attention_heads: gnn_cfg.attention_heads,
        gnn_max_nodes: gnn_cfg.max_nodes,
        gnn_use_residual: gnn_cfg.use_residual,
        gnn_use_layer_norm: gnn_cfg.use_layer_norm,
        gnn_activation: gnn_cfg.activation.clone(),
        gnn_weight_seed: gnn_cfg.weight_seed,
    };
    archon_pipeline::learning::gnn::auto_trainer_runtime::build_and_spawn_auto_trainer(params, db)
}

#[cfg(test)]
#[path = "pipeline_support_tests.rs"]
mod pipeline_support_tests;
