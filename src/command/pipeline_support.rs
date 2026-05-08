use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use archon_core::config::ArchonConfig;
use archon_core::env_vars::ArchonEnvVars;
use archon_pipeline::audit::store::PipelineBundleStore;
use archon_pipeline::audit::types::PipelineEvent;
use archon_pipeline::leann_searcher::LeannSearcher;
use chrono::Utc;

use crate::runtime::llm::build_configured_llm_provider;

struct CodeIndexLeannSearcher {
    index: Arc<archon_leann::CodeIndex>,
}

impl LeannSearcher for CodeIndexLeannSearcher {
    fn search(&self, query: &str) -> String {
        match self.index.search_code(query, 5) {
            Ok(results) if !results.is_empty() => {
                let mut out = String::with_capacity(results.len() * 256);
                for r in &results {
                    let snippet: String = r.content.chars().take(300).collect();
                    out.push_str(&format!(
                        "{}:{}  {}\n",
                        r.file_path.display(),
                        r.line_start,
                        snippet
                    ));
                }
                out
            }
            _ => String::new(),
        }
    }
}

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

pub(crate) async fn init_research_leann(cwd: &Path) -> Option<Arc<dyn LeannSearcher>> {
    let db_path = cwd.join(".archon").join("leann.db");
    if let Some(parent) = db_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match archon_leann::CodeIndex::new(&db_path, Default::default()) {
        Ok(idx) => {
            let idx = Arc::new(idx);
            let config = archon_leann::IndexConfig {
                root_path: cwd.to_path_buf(),
                include_patterns: vec!["**/*.rs".into(), "**/*.py".into(), "**/*.ts".into()],
                exclude_patterns: vec![
                    "**/target/**".into(),
                    "**/node_modules/**".into(),
                    "**/.git/**".into(),
                ],
            };
            if let Err(e) = idx.index_repository(cwd, &config).await {
                tracing::warn!(error = %e, "LEANN repo indexing failed; research pipeline continuing without code context");
            }
            Some(Arc::new(CodeIndexLeannSearcher { index: idx }))
        }
        Err(e) => {
            tracing::warn!(error = %e, "LEANN unavailable for research pipeline; continuing without code context");
            None
        }
    }
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
    match completion_summary(result, cwd).await {
        Ok(Some(summary)) => println!("Completion integrity: {}", summary.text),
        Ok(None) => {}
        Err(error) => {
            println!("Completion integrity: unavailable ({error})");
        }
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
    let data_dir = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from(".local/share"))
        .join("archon");
    std::fs::create_dir_all(&data_dir)?;
    let path = data_dir.join("archon-data.db");
    let path_str = path.to_string_lossy().to_string();
    let db = cozo::DbInstance::new("sqlite", &path_str, "")
        .map_err(|e| anyhow::anyhow!("open completion store at {path_str}: {e}"))?;
    let task_type = match result.pipeline_type {
        archon_pipeline::runner::PipelineType::Coding => "coding",
        archon_pipeline::runner::PipelineType::Research => "research",
        archon_pipeline::runner::PipelineType::Learning => "learning",
        archon_pipeline::runner::PipelineType::Kb => "kb",
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

pub(crate) fn build_pipeline_auto_trainer(
    config: &ArchonConfig,
    cwd: &Path,
) -> Option<Arc<archon_pipeline::learning::gnn::auto_trainer::AutoTrainer>> {
    let at_cfg = &config.learning.gnn.auto_trainer;
    if !at_cfg.enabled || !config.learning.gnn.enabled {
        return None;
    }

    let db_path = cwd.join(".archon").join("learning.db");
    if let Some(parent) = db_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let db = match cozo::DbInstance::new("newrocksdb", db_path.to_str().unwrap_or(""), "") {
        Ok(db) => db,
        Err(e) => {
            tracing::warn!(error = %e, "pipeline: learning DB unavailable; auto_trainer not spawned");
            return None;
        }
    };
    if let Err(e) = archon_pipeline::learning::schema::initialize_learning_schemas(&db) {
        tracing::warn!(error = %e, "pipeline: learning schema init failed; auto_trainer not spawned");
        return None;
    }
    let db = Arc::new(db);

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
    archon_pipeline::learning::gnn::auto_trainer_runtime::build_and_spawn_auto_trainer(params, db)
}
