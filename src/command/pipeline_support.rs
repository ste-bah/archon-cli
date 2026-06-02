use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use archon_core::config::ArchonConfig;
use archon_core::env_vars::ArchonEnvVars;
use archon_pipeline::audit::store::PipelineBundleStore;
use archon_pipeline::audit::types::PipelineEvent;
use archon_pipeline::learning::causal::CausalMemory;
use archon_pipeline::learning::desc::DescEpisodeStore;
use archon_pipeline::learning::integration::{LearningIntegration, LearningIntegrationConfig};
use archon_pipeline::learning::patterns::PatternStore;
use archon_pipeline::learning::reasoning::{ReasoningBank, ReasoningBankConfig, ReasoningBankDeps};
use archon_pipeline::learning::reflexion::ReflexionInjector;
use archon_pipeline::runner::PipelineType;
use chrono::Utc;

use crate::runtime::llm::build_configured_llm_provider;

pub(crate) async fn build_pipeline_adapter(
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
    origin: &str,
) -> Result<archon_pipeline::llm_adapter::ProviderLlmAdapter> {
    let provider = build_configured_llm_provider(config, env_vars, origin).await?;
    Ok(archon_pipeline::llm_adapter::ProviderLlmAdapter::new(provider).with_origin(origin))
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

pub(crate) async fn print_pipeline_result(
    result: &archon_pipeline::runner::PipelineResult,
    cwd: &Path,
) {
    println!("\n=== Pipeline Complete ===");
    println!("Session: {}", result.session_id);
    println!("Agents run: {}", result.agent_results.len());
    println!("Total cost: ${:.4}", result.total_cost_usd);
    println!("Duration: {:.1}s", result.duration.as_secs_f64());
    if let Some((markdown_path, pdf_path)) = final_research_artifact_paths(result, cwd) {
        println!("Final paper Markdown: {}", markdown_path.display());
        println!("Final paper PDF: {}", pdf_path.display());
    }
    match completion_summary(result, cwd).await {
        Ok(Some(summary)) => println!("Completion integrity: {}", summary.text),
        Ok(None) => {}
        Err(error) => {
            println!("Completion integrity: unavailable ({error})");
        }
    }
}

pub(crate) fn final_research_artifact_paths(
    result: &archon_pipeline::runner::PipelineResult,
    cwd: &Path,
) -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    if result.pipeline_type != PipelineType::Research {
        return None;
    }
    let bundle_dir = PipelineBundleStore::new(cwd).bundle_dir(&result.session_id);
    let (markdown_path, pdf_path) =
        archon_pipeline::research::final_artifact::artifact_paths(&bundle_dir);
    if markdown_path.exists() || pdf_path.exists() {
        Some((markdown_path, pdf_path))
    } else {
        None
    }
}

struct CompletionSummary {
    text: String,
}

async fn completion_summary(
    result: &archon_pipeline::runner::PipelineResult,
    cwd: &Path,
) -> Result<Option<CompletionSummary>> {
    if result.final_output.trim().is_empty() {
        return Ok(None);
    }
    if result.pipeline_type == PipelineType::Research {
        let store = PipelineBundleStore::new(cwd);
        if let Ok(mut state) = store.load_state(&result.session_id) {
            state.completion_integrity_summary = None;
            state.completion_report_id = None;
            state.updated_at = Utc::now();
            store.save_state(&state)?;
        }
        return Ok(None);
    }
    let db = crate::command::store_paths::open_evidence_db(
        "completion",
        &["ARCHON_COMPLETION_DB_PATH"],
    )?;
    let task_type = match result.pipeline_type {
        archon_pipeline::runner::PipelineType::Coding => "coding",
        archon_pipeline::runner::PipelineType::Research => "research",
        archon_pipeline::runner::PipelineType::Learning => "learning",
        archon_pipeline::runner::PipelineType::Kb => "kb",
        archon_pipeline::runner::PipelineType::GameTheory => "gametheory",
        archon_pipeline::runner::PipelineType::Workflow => "workflow",
    };
    let (agent_key, model) = result
        .agent_results
        .last()
        .map(|(agent, _)| (Some(agent.key.clone()), Some(agent.model.clone())))
        .unwrap_or((None, None));
    let report = archon_completion::check_completion_with_context(
        &db,
        &result.session_id,
        &result.final_output,
        task_type,
        archon_completion::CompletionContext {
            workspace_id: std::env::current_dir()
                .ok()
                .map(|path| path.to_string_lossy().to_string())
                .unwrap_or_else(|| "default".into()),
            agent_key,
            model,
        },
    )
    .await?;
    let verified = report.claims.iter().filter(|claim| claim.verified).count();
    let summary = format!(
        "{:?}; {verified}/{} completion-sensitive claims verified",
        report.final_state,
        report.claims.len()
    );
    let store = PipelineBundleStore::new(cwd);
    if let Ok(mut state) = store.load_state(&result.session_id) {
        state.completion_integrity_summary = Some(summary.clone());
        state.completion_report_id = Some(report.report_id.clone());
        state.updated_at = Utc::now();
        store.save_state(&state)?;
        store.append_event(
            &result.session_id,
            PipelineEvent::CompletionChecked {
                final_state: format!("{:?}", report.final_state),
                claim_count: report.claims.len(),
                verified_claim_count: verified,
                report_id: report.report_id,
            },
        )?;
    }
    Ok(Some(CompletionSummary { text: summary }))
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
    let db_path =
        crate::command::store_paths::evidence_db_path_for_dir(cwd, &["ARCHON_LEARNING_DB_PATH"]);
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
    Some(Arc::new(db))
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
mod tests {
    use super::*;

    #[test]
    fn pipeline_learning_schema_can_share_sqlite_evidence_store() {
        let temp = tempfile::tempdir().expect("tempdir");
        let db_path = temp.path().join(".archon").join("archon-data.db");
        let db = open_pipeline_learning_db_at(temp.path(), &db_path).expect("pipeline db");

        archon_learning::schema::ensure_learning_schema(db.as_ref())
            .expect("governed learning schema");

        assert!(db_path.exists());
        assert!(!temp.path().join(".archon").join("learning.db").exists());
    }
}
