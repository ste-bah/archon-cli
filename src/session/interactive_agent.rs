use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::cli_args::Cli;
use anyhow::Result;
use archon_core::agent::{Agent, AgentConfig, TimestampedEvent};
use archon_core::agents::AgentRegistry;
use archon_core::tasks::TaskService;
use archon_llm::anthropic::AnthropicClient;
use archon_memory::MemoryTrait;
use archon_observability::ChannelMetricSink;
use archon_tui::event_channel::{TuiEventReceiver, TuiEventSender};
use archon_tui::observability;

use crate::runtime::llm::{
    build_llm_provider_selection, provider_construction_error_reason,
    record_anthropic_fallback_denied,
};
use crate::runtime::llm_non_anthropic::build_llm_provider_without_anthropic_fallback;
use crate::runtime::provider_observer::{
    observe_llm_provider_with_profile, record_provider_fallback, runtime_mode_for_provider_name,
};

pub(super) struct Runtime {
    pub agent: Agent,
    pub provider: Arc<dyn archon_llm::provider::LlmProvider>,
    pub agent_event_rx: tokio::sync::mpsc::UnboundedReceiver<TimestampedEvent>,
    pub tui_event_tx: TuiEventSender,
    pub tui_event_rx: TuiEventReceiver,
    pub user_input_tx: tokio::sync::mpsc::Sender<String>,
    pub user_input_rx: tokio::sync::mpsc::Receiver<String>,
    pub agent_registry_for_skills: Arc<std::sync::RwLock<AgentRegistry>>,
    pub task_service: Arc<dyn TaskService>,
    pub coding_pipeline: Arc<archon_pipeline::coding::facade::CodingFacade>,
    pub research_pipeline: Arc<archon_pipeline::research::facade::ResearchFacade>,
    pub llm_adapter: Arc<dyn archon_pipeline::runner::LlmClient>,
    pub leann: Option<Arc<archon_pipeline::runner::LeannIntegration>>,
    pub leann_init_cancel: Arc<AtomicBool>,
    pub learning_cozo_db: Option<Arc<cozo::DbInstance>>,
    pub governed_learning_db: Option<Arc<cozo::DbInstance>>,
    pub auto_trainer: Option<Arc<archon_pipeline::learning::gnn::auto_trainer::AutoTrainer>>,
    pub metrics: Arc<archon_tui::observability::ChannelMetrics>,
    pub agent_event_tx_for_dispatcher: tokio::sync::mpsc::UnboundedSender<TimestampedEvent>,
}

pub(super) async fn build(
    config: &archon_core::config::ArchonConfig,
    session_id: &str,
    cli: &Cli,
    working_dir: PathBuf,
    hook_registry: Arc<archon_core::hooks::HookRegistry>,
    provider_override: Option<Arc<dyn archon_llm::provider::LlmProvider>>,
    anthropic_client: Option<AnthropicClient>,
    memory: Arc<dyn MemoryTrait>,
    session_store: Arc<archon_session::storage::SessionStore>,
    checkpoint_store: Option<archon_session::checkpoint::CheckpointStore>,
    mut agent_config: AgentConfig,
    registry: archon_core::dispatch::ToolRegistry,
    voice_event_rx: Option<tokio::sync::mpsc::UnboundedReceiver<archon_tui::app::TuiEvent>>,
) -> Result<Runtime> {
    let (agent_event_tx, agent_event_rx) =
        tokio::sync::mpsc::unbounded_channel::<TimestampedEvent>();
    let (tui_event_tx, tui_event_rx) = archon_tui::event_channel::bounded_tui_event_channel();
    agent_config.activity_sink =
        super::session_activity_sink_with_tui(session_id, tui_event_tx.clone());

    observability::spawn_named("tui-drain-stall-detector", async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
        loop {
            interval.tick().await;
            let threshold = archon_tui::observability::current_drain_threshold_ms();
            archon_tui::observability::warn_if_drain_stalled(threshold);
        }
    });

    if let Some(mut voice_rx) = voice_event_rx {
        let voice_fwd_tx = tui_event_tx.clone();
        observability::spawn_named("voice-event-forwarder", async move {
            while let Some(evt) = voice_rx.recv().await {
                if voice_fwd_tx.send(evt).is_err() {
                    break;
                }
            }
        });
    }

    let (user_input_tx, user_input_rx) = tokio::sync::mpsc::channel::<String>(16);
    let provider = resolve_provider(
        config,
        session_id,
        &working_dir,
        &hook_registry,
        provider_override,
        anthropic_client,
    )
    .await?;

    let agent_registry = Arc::new(std::sync::RwLock::new(AgentRegistry::load(&working_dir)));
    {
        let reg = agent_registry.read().expect("agent registry lock");
        tracing::info!(count = reg.len(), "loaded agent definitions");
        for err in reg.load_errors() {
            tracing::warn!(%err, "agent load error");
        }
    }
    let agent_registry_for_skills = Arc::clone(&agent_registry);

    let task_service: Arc<dyn TaskService> = Arc::new(archon_core::tasks::DefaultTaskService::new(
        Arc::new(archon_core::agents::AgentRegistry::load(&working_dir)),
        10000,
    ));

    let leann_init_cancel = Arc::new(AtomicBool::new(false));
    let leann: Option<Arc<archon_pipeline::runner::LeannIntegration>> = {
        let db_path = working_dir.join(".archon").join("leann.db");
        if let Some(parent) = db_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match archon_leann::CodeIndex::new(&db_path, Default::default()) {
            Ok(idx) => {
                let li = Arc::new(archon_pipeline::runner::LeannIntegration::new(
                    std::sync::Arc::new(idx),
                ));
                let li_bg = Arc::clone(&li);
                let wd = working_dir.clone();
                let leann_cancel = Arc::clone(&leann_init_cancel);
                observability::spawn_named("leann-background-init", async move {
                    let leann_cancel_for_blocking = Arc::clone(&leann_cancel);
                    let result =
                        observability::spawn_blocking_named("leann-background-index", move || {
                            li_bg.init_repository_blocking_with_cancel(
                                &wd,
                                leann_cancel_for_blocking.as_ref(),
                            )
                        })
                        .await;
                    match result {
                        Ok(Ok(())) => {}
                        Ok(Err(e)) => {
                            tracing::warn!(error = %e, "LEANN background init failed; continuing without code context");
                        }
                        Err(e) if e.is_cancelled() => {
                            tracing::info!("LEANN background init cancelled");
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "LEANN background init join failed; continuing without code context");
                        }
                    }
                });
                Some(li)
            }
            Err(e) => {
                tracing::warn!(error = %e, "LEANN unavailable; continuing without code context");
                None
            }
        }
    };

    let learning_cozo_db = {
        let db_path = crate::command::store_paths::evidence_db_path_for_dir(
            &working_dir,
            &["ARCHON_LEARNING_DB_PATH"],
        );
        if let Some(parent) = db_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match cozo::DbInstance::new("sqlite", db_path.to_str().unwrap_or(""), "") {
            Ok(db) => {
                if let Err(e) = archon_pipeline::learning::schema::initialize_learning_schemas(&db)
                {
                    tracing::warn!(error = %e, "Learning schema init failed; retrain may not work");
                } else {
                    crate::command::pipeline_learning_migration::maybe_migrate_legacy_pipeline_learning_with_log(
                        &working_dir,
                        &db_path,
                        &db,
                        "interactive",
                    );
                }
                Some(Arc::new(db))
            }
            Err(e) => {
                tracing::warn!(error = %e, "CozoDB learning store unavailable; retrain disabled");
                None
            }
        }
    };
    let governed_learning_db = super::open_governed_learning_db(&working_dir);

    let auto_trainer = build_auto_trainer(config, &learning_cozo_db);

    let coding_pipeline_facade = if let Some(db) = learning_cozo_db.as_ref()
        && config.learning.sona.enabled
    {
        archon_pipeline::coding::facade::CodingFacade::with_learning(
            archon_pipeline::learning::integration::LearningIntegration::new_with_persistent_sona(
                Arc::clone(db),
                Default::default(),
                auto_trainer.clone(),
                config.learning.gnn.input_dim,
            ),
        )
    } else {
        archon_pipeline::coding::facade::CodingFacade::new()
    };
    let coding_pipeline: Arc<archon_pipeline::coding::facade::CodingFacade> = Arc::new(
        coding_pipeline_facade
            .with_models(config.models.anthropic.clone())
            .with_context(config.context.clone()),
    );
    let research_pipeline: Arc<archon_pipeline::research::facade::ResearchFacade> = Arc::new(
        archon_pipeline::research::facade::ResearchFacade::new(
            Arc::clone(&memory),
            None,
            working_dir.display().to_string(),
            None,
        )
        .with_models(config.models.anthropic.clone())
        .with_context(config.context.clone()),
    );
    let llm_adapter: Arc<dyn archon_pipeline::runner::LlmClient> = Arc::new(
        archon_pipeline::llm_adapter::ProviderLlmAdapter::new(Arc::clone(&provider))
            .with_origin("tui_pipeline"),
    );
    let agent_event_tx_for_dispatcher = agent_event_tx.clone();
    let mut agent = Agent::new(
        Arc::clone(&provider),
        registry,
        agent_config,
        agent_event_tx,
        agent_registry,
    );
    let metrics = Arc::new(archon_tui::observability::ChannelMetrics::default());
    let metrics_sink: Arc<dyn ChannelMetricSink> = metrics.clone();
    agent.set_channel_metrics(metrics_sink);

    super::spawn_metrics_exporter(cli.metrics_port, Arc::clone(&metrics))?;

    if let Some(store) = checkpoint_store {
        agent.set_checkpoint_store(store);
    }
    if let Ok(plan_store) = archon_session::plan::PlanStore::new(session_store.db()) {
        agent.set_plan_store(plan_store);
        tracing::info!("plan store wired into agent");
    } else {
        tracing::warn!("failed to initialize plan store");
    }
    if config.memory.enabled {
        agent.set_memory(Arc::clone(&memory));
    }
    if config.memory.auto_extraction.enabled && config.memory.enabled {
        let extractor = Arc::new(archon_core::auto_extraction::AutoExtractor::new(
            Arc::clone(&provider),
            Arc::clone(&memory),
            config.memory.auto_extraction.every_n_turns,
            true,
        ));
        agent.set_auto_extractor(extractor);
        tracing::info!(
            every_n_turns = config.memory.auto_extraction.every_n_turns,
            "auto-extraction: wired into agent loop"
        );
    }

    if let Some(ref at) = auto_trainer {
        let at_mem = Arc::clone(at);
        let mem_cb: Arc<dyn Fn(u64) + Send + Sync> = Arc::new(move |n| at_mem.record_memories(n));
        agent.set_record_memory_callback(mem_cb);

        let at_corr = Arc::clone(at);
        let corr_cb: Arc<dyn Fn() + Send + Sync> = Arc::new(move || at_corr.record_correction());
        agent.set_record_correction_callback(corr_cb);
    }

    let correction_learning = governed_learning_db.as_ref().map(|db| {
        Arc::new(
            archon_pipeline::learning::integration::LearningIntegration::new(
                None,
                None,
                Default::default(),
                auto_trainer.clone(),
            )
            .with_event_store(Arc::clone(db)),
        )
    });
    super::reasoning_quality::wire_callbacks(
        &mut agent,
        config,
        session_id,
        &working_dir,
        governed_learning_db.clone(),
        correction_learning,
        Arc::clone(&provider),
    );

    Ok(Runtime {
        agent,
        provider,
        agent_event_rx,
        tui_event_tx,
        tui_event_rx,
        user_input_tx,
        user_input_rx,
        agent_registry_for_skills,
        task_service,
        coding_pipeline,
        research_pipeline,
        llm_adapter,
        leann,
        leann_init_cancel,
        learning_cozo_db,
        governed_learning_db,
        auto_trainer,
        metrics,
        agent_event_tx_for_dispatcher,
    })
}

async fn resolve_provider(
    config: &archon_core::config::ArchonConfig,
    session_id: &str,
    working_dir: &std::path::Path,
    hook_registry: &Arc<archon_core::hooks::HookRegistry>,
    provider_override: Option<Arc<dyn archon_llm::provider::LlmProvider>>,
    anthropic_client: Option<AnthropicClient>,
) -> Result<Arc<dyn archon_llm::provider::LlmProvider>> {
    let provider_was_prebuilt = provider_override.is_some();
    if !provider_was_prebuilt {
        crate::runtime::hooks::fire_provider_resolve_hook(
            hook_registry,
            working_dir,
            session_id,
            crate::runtime::hooks::ProviderResolveHookPayload {
                hook_event: "BeforeProviderResolve",
                stage: "before_provider_resolve",
                surface: "interactive_session",
                requested_provider: &config.llm.provider,
                selected_provider: None,
                runtime_mode: None,
                profile_id: None,
            },
        )
        .await;
    }

    let provider = match provider_override {
        Some(provider) => provider,
        None => match anthropic_client {
            Some(client) => {
                let selection = build_llm_provider_selection(&config.llm, &config.models, client);
                let selected_provider = selection.provider.name().to_string();
                let runtime_mode = runtime_mode_for_provider_name(&selected_provider);
                record_provider_fallback(
                    &config.llm.provider,
                    &selected_provider,
                    runtime_mode,
                    selection
                        .fallback_reason
                        .unwrap_or("provider_construction_fallback"),
                );
                let profile_id =
                    crate::runtime::provider_auth_selection::selected_provider_auth_profile_id(
                        &selected_provider,
                    );
                observe_llm_provider_with_profile(selection.provider, runtime_mode, profile_id)
            }
            None => {
                let provider = build_llm_provider_without_anthropic_fallback(&config.llm).map_err(
                    |error| {
                        let reason = provider_construction_error_reason(&error);
                        record_anthropic_fallback_denied(
                            &config.llm.provider,
                            "interactive_session",
                            reason,
                        );
                        anyhow::anyhow!("provider {} failed: {error}", config.llm.provider)
                    },
                )?;
                let selected_provider = provider.name().to_string();
                let runtime_mode = runtime_mode_for_provider_name(&selected_provider);
                let profile_id =
                    crate::runtime::provider_auth_selection::selected_provider_auth_profile_id(
                        &selected_provider,
                    );
                observe_llm_provider_with_profile(provider, runtime_mode, profile_id)
            }
        },
    };

    if !provider_was_prebuilt {
        let selected_provider = provider.name().to_string();
        let profile_id = crate::runtime::provider_auth_selection::selected_provider_auth_profile_id(
            &selected_provider,
        );
        crate::runtime::hooks::fire_provider_resolve_hook(
            hook_registry,
            working_dir,
            session_id,
            crate::runtime::hooks::ProviderResolveHookPayload {
                hook_event: "AfterProviderResolve",
                stage: "after_provider_resolve",
                surface: "interactive_session",
                requested_provider: &config.llm.provider,
                selected_provider: Some(&selected_provider),
                runtime_mode: Some(runtime_mode_for_provider_name(&selected_provider)),
                profile_id: profile_id.as_deref(),
            },
        )
        .await;
    }
    tracing::info!("LLM provider: {}", provider.name());
    Ok(provider)
}

fn build_auto_trainer(
    config: &archon_core::config::ArchonConfig,
    learning_cozo_db: &Option<Arc<cozo::DbInstance>>,
) -> Option<Arc<archon_pipeline::learning::gnn::auto_trainer::AutoTrainer>> {
    let at_cfg = &config.learning.gnn.auto_trainer;
    if !at_cfg.enabled || !config.learning.gnn.enabled {
        tracing::info!(
            at_enabled = at_cfg.enabled,
            gnn_enabled = config.learning.gnn.enabled,
            "GNN auto-trainer disabled by config"
        );
        return None;
    }
    let Some(db) = learning_cozo_db.as_ref() else {
        tracing::warn!(
            "GNN auto-trainer enabled in config but learning CozoDB unavailable; not spawning"
        );
        return None;
    };

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
    let auto_trainer =
        archon_pipeline::learning::gnn::auto_trainer_runtime::build_and_spawn_auto_trainer(
            params,
            Arc::clone(db),
        );
    if auto_trainer.is_some() {
        tracing::info!(
            interval_ms = at_cfg.tick_interval_ms,
            throttle_ms = at_cfg.min_throttle_ms,
            first_run_threshold = at_cfg.first_run_threshold,
            "GNN auto-trainer spawned"
        );
    }
    auto_trainer
}
