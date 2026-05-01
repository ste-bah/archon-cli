//! `/pipeline` slash command handler.
//! Extracted from main.rs to reduce main.rs from 1532 to < 500 lines.

use std::path::PathBuf;
use std::sync::Arc;

use archon_core::config::ArchonConfig;
use archon_core::env_vars::ArchonEnvVars;
use archon_llm::auth::resolve_auth_with_keys;
use archon_llm::identity::{IdentityMode, IdentityProvider};
use archon_memory::{MemoryTrait, graph::MemoryGraph};
use archon_pipeline::coding::rlm::LeannSearcher;

use crate::cli_args::PipelineAction;

/// LEANN searcher backed by a real [`archon_leann::CodeIndex`].
///
/// Used by the research pipeline so it gets actual semantic code search
/// results instead of the prior `NoopLeannSearcher` ghost-wire (GHOST-009).
struct CodeIndexLeannSearcher {
    index: std::sync::Arc<archon_leann::CodeIndex>,
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

/// Handle `/archon pipeline` subcommands.
pub async fn handle_pipeline_command(
    action: &PipelineAction,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
) -> std::result::Result<(), anyhow::Error> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    match action {
        PipelineAction::Code { task, dry_run } => {
            if *dry_run {
                println!("=== Coding Pipeline Dry Run ===");
                println!("Task: {task}");
                println!("\nAgent Sequence (48 agents):");
                println!(
                    "  Phase 1: task-analyzer, requirement-extractor, requirement-prioritizer"
                );
                println!(
                    "  Phase 2: pattern-explorer, technology-scout, feasibility-analyzer, codebase-analyzer"
                );
                println!("  Phase 3: system-designer, component-designer, interface-designer, ...");
                println!("  Phase 4: code-generator, unit-implementer, api-implementer, ...");
                println!("  Phase 5: test-generator, integration-tester, security-tester, ...");
                println!("  Phase 6: final-refactorer, sign-off-approver");
                println!("\nEstimated cost: ~$2.50-5.00 (varies by task complexity)");
            } else {
                let pipe_auth = resolve_auth_with_keys(
                    env_vars.anthropic_api_key.as_deref(),
                    env_vars.archon_api_key.as_deref(),
                    env_vars.archon_oauth_token.as_deref(),
                    std::env::var("ANTHROPIC_AUTH_TOKEN").ok().as_deref(),
                )
                .map_err(|e| anyhow::anyhow!("Authentication failed: {e}"))?;
                let identity = IdentityProvider::new(
                    IdentityMode::Clean,
                    uuid::Uuid::new_v4().to_string(),
                    "pipeline-device".to_string(),
                    String::new(),
                );
                let api_url = std::env::var("ANTHROPIC_BASE_URL")
                    .ok()
                    .or_else(|| config.api.base_url.clone());
                let pipe_client =
                    archon_llm::anthropic::AnthropicClient::new(pipe_auth, identity, api_url);
                let adapter = archon_pipeline::llm_adapter::AnthropicLlmAdapter::new(
                    std::sync::Arc::new(pipe_client),
                );
                let learning = archon_pipeline::learning::integration::LearningIntegration::new(
                    None,
                    None,
                    Default::default(),
                    None,
                );
                let facade = archon_pipeline::coding::facade::CodingFacade::with_learning(learning);
                let leann = init_leann(&cwd).await;
                println!("Starting coding pipeline...");
                println!("Task: {task}");
                let result = archon_pipeline::runner::run_pipeline(
                    &facade,
                    &adapter,
                    task,
                    leann.as_ref(),
                    None,
                    None,
                )
                .await?;
                print_pipeline_result(&result);
            }
        }
        PipelineAction::Research { topic, dry_run } => {
            if *dry_run {
                println!("=== Research Pipeline Dry Run ===");
                println!("Topic: {topic}");
                println!("\nAgent Sequence (46 agents):");
                println!("  Phase 1-3: foundation, analysis, methodology");
                println!("  Phase 4-7: writing, verification");
                println!("  Phase 8: final-stage-orchestrator");
                println!("\nEstimated cost: ~$3.00-8.00 (varies by topic complexity)");
            } else {
                let pipe_auth = resolve_auth_with_keys(
                    env_vars.anthropic_api_key.as_deref(),
                    env_vars.archon_api_key.as_deref(),
                    env_vars.archon_oauth_token.as_deref(),
                    std::env::var("ANTHROPIC_AUTH_TOKEN").ok().as_deref(),
                )
                .map_err(|e| anyhow::anyhow!("Authentication failed: {e}"))?;
                let identity = IdentityProvider::new(
                    IdentityMode::Clean,
                    uuid::Uuid::new_v4().to_string(),
                    "pipeline-device".to_string(),
                    String::new(),
                );
                let api_url = std::env::var("ANTHROPIC_BASE_URL")
                    .ok()
                    .or_else(|| config.api.base_url.clone());
                let pipe_client =
                    archon_llm::anthropic::AnthropicClient::new(pipe_auth, identity, api_url);
                let adapter = archon_pipeline::llm_adapter::AnthropicLlmAdapter::new(
                    std::sync::Arc::new(pipe_client),
                );
                let phd_learning =
                    archon_pipeline::learning::integration::PhDLearningIntegration::new();
                let memory: Arc<dyn MemoryTrait> = Arc::new(
                    MemoryGraph::in_memory().expect("in-memory graph for research pipeline"),
                );
                let leann = init_research_leann(&cwd).await;
                let facade = archon_pipeline::research::facade::ResearchFacade::with_learning(
                    memory,
                    leann,
                    cwd.display().to_string(),
                    None,
                    phd_learning,
                );
                println!("Starting research pipeline...");
                println!("Topic: {topic}");
                let result = archon_pipeline::runner::run_pipeline(
                    &facade, &adapter, topic, None, None, None,
                )
                .await?;
                print_pipeline_result(&result);
            }
        }
        PipelineAction::Status { session_id } => {
            let cp_path = cwd
                .join(".pipeline-state")
                .join(session_id)
                .join("checkpoint.json");
            match std::fs::read_to_string(&cp_path) {
                Ok(data) => {
                    let session: archon_pipeline::session::PipelineCheckpoint =
                        serde_json::from_str(&data).unwrap_or_else(|e| {
                            eprintln!("Failed to parse checkpoint: {e}");
                            std::process::exit(1);
                        });
                    println!("Session: {}", session.session_id);
                    println!("Type: {:?}", session.pipeline_type);
                    println!("Task: {}", session.task);
                    println!("Status: {:?}", session.status);
                    println!("Completed agents: {}", session.completed_agents.len());
                    println!("Total cost: ${:.4}", session.total_cost_usd);
                    println!("Started: {}", session.started_at);
                    println!("Updated: {}", session.updated_at);
                    for agent in &session.completed_agents {
                        println!(
                            "  {} (quality: {:.2}, cost: ${:.4})",
                            agent.agent_key, agent.quality_score, agent.cost_usd
                        );
                    }
                }
                Err(e) => {
                    eprintln!("Failed to load session {session_id}: {e}");
                    std::process::exit(1);
                }
            }
        }
        PipelineAction::Resume { session_id } => {
            match archon_pipeline::session::resume(session_id, &cwd) {
                Ok(session) => {
                    println!("Resumed session: {}", session.session_id);
                    println!("Completed agents: {}", session.completed_agents.len());
                    let pipe_auth = resolve_auth_with_keys(
                        env_vars.anthropic_api_key.as_deref(),
                        env_vars.archon_api_key.as_deref(),
                        env_vars.archon_oauth_token.as_deref(),
                        std::env::var("ANTHROPIC_AUTH_TOKEN").ok().as_deref(),
                    )
                    .map_err(|e| anyhow::anyhow!("Authentication failed: {e}"))?;
                    let identity = IdentityProvider::new(
                        IdentityMode::Clean,
                        uuid::Uuid::new_v4().to_string(),
                        "pipeline-device".to_string(),
                        String::new(),
                    );
                    let api_url = std::env::var("ANTHROPIC_BASE_URL")
                        .ok()
                        .or_else(|| config.api.base_url.clone());
                    let pipe_client =
                        archon_llm::anthropic::AnthropicClient::new(pipe_auth, identity, api_url);
                    let adapter = archon_pipeline::llm_adapter::AnthropicLlmAdapter::new(
                        std::sync::Arc::new(pipe_client),
                    );
                    let facade_type = &session.pipeline_type;
                    match format!("{:?}", facade_type).as_str() {
                        "Coding" => {
                            // Reference: archon-pipeline/src/learning/gnn/auto_trainer_runtime.rs.
                            // Same wiring as the fresh `Code` action above; resume path
                            // gets the same auto-trainer treatment so the loop is consistent.
                            let resume_auto_trainer = build_pipeline_auto_trainer(config, &cwd);
                            let learning =
                                archon_pipeline::learning::integration::LearningIntegration::new(
                                    None,
                                    None,
                                    Default::default(),
                                    resume_auto_trainer,
                                );
                            let facade =
                                archon_pipeline::coding::facade::CodingFacade::with_learning(
                                    learning,
                                );
                            println!("Resuming coding pipeline...");
                            let result = archon_pipeline::runner::run_pipeline(
                                &facade,
                                &adapter,
                                &session.task,
                                None,
                                None,
                                None,
                            )
                            .await?;
                            print_pipeline_result(&result);
                        }
                        "Research" => {
                            let phd_learning =
                                archon_pipeline::learning::integration::PhDLearningIntegration::new(
                                );
                            let memory: Arc<dyn MemoryTrait> = Arc::new(
                                MemoryGraph::in_memory()
                                    .expect("in-memory graph for research resume"),
                            );
                            let leann = init_research_leann(&cwd).await;
                            let facade =
                                archon_pipeline::research::facade::ResearchFacade::with_learning(
                                    memory,
                                    leann,
                                    cwd.display().to_string(),
                                    None,
                                    phd_learning,
                                );
                            println!("Resuming research pipeline...");
                            let result = archon_pipeline::runner::run_pipeline(
                                &facade,
                                &adapter,
                                &session.task,
                                None,
                                None,
                                None,
                            )
                            .await?;
                            print_pipeline_result(&result);
                        }
                        other => {
                            eprintln!("Unknown pipeline type: {other}");
                            std::process::exit(1);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Failed to resume session {session_id}: {e}");
                    std::process::exit(1);
                }
            }
        }
        PipelineAction::List => match archon_pipeline::session::list_sessions(&cwd) {
            Ok(sessions) if sessions.is_empty() => {
                println!("No pipeline sessions found.");
            }
            Ok(sessions) => {
                println!(
                    "{:<38} {:<10} {:<10} {:>6} {:>10}  TASK",
                    "SESSION ID", "TYPE", "STATUS", "AGENTS", "COST"
                );
                println!("{}", "-".repeat(100));
                for s in &sessions {
                    let truncated: String = s.task.chars().take(30).collect();
                    println!(
                        "{:<38} {:<10} {:<10} {:>6} ${:>9.4}  {}",
                        s.session_id,
                        format!("{:?}", s.pipeline_type),
                        format!("{:?}", s.status),
                        s.completed_count,
                        s.total_cost_usd,
                        truncated,
                    );
                }
            }
            Err(e) => {
                eprintln!("Failed to list sessions: {e}");
                std::process::exit(1);
            }
        },
        PipelineAction::Abort { session_id } => {
            match archon_pipeline::session::abort(session_id, &cwd) {
                Ok(()) => println!("Session {session_id} aborted."),
                Err(e) => {
                    eprintln!("Failed to abort session {session_id}: {e}");
                    std::process::exit(1);
                }
            }
        }
        PipelineAction::Run {
            file,
            format,
            detach,
        } => {
            let src = match std::fs::read_to_string(file) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Failed to read {}: {e}", file.display());
                    std::process::exit(3);
                }
            };
            let fmt = detect_format(file, format.as_deref(), &src)?;
            let store_path = cwd.join(".archon").join("pipelines");
            let _ = std::fs::create_dir_all(&store_path);
            let store = std::sync::Arc::new(archon_pipeline::PipelineStateStore::new(&store_path));
            let registry = std::sync::Arc::new(archon_core::agents::AgentRegistry::load(&cwd));
            let task_service: std::sync::Arc<dyn archon_core::tasks::TaskService> =
                std::sync::Arc::new(archon_core::tasks::DefaultTaskService::new(registry, 10000));
            let engine = archon_pipeline::DefaultPipelineEngine::new(store, task_service);
            let spec = match engine.parse(&src, fmt) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Parse error: {e}");
                    std::process::exit(3);
                }
            };
            if let Err(e) = engine.validate(&spec) {
                eprintln!("Validation error: {e}");
                std::process::exit(3);
            }
            use archon_pipeline::PipelineEngine;
            match engine.run(spec).await {
                Ok(id) => {
                    println!("{id}");
                    if !detach {
                        poll_pipeline_status(engine, id).await;
                    }
                }
                Err(e) => {
                    eprintln!("Pipeline failed: {e}");
                    std::process::exit(1);
                }
            }
        }
        PipelineAction::Cancel { id } => {
            let pipeline_id: archon_pipeline::PipelineId = match id.parse() {
                Ok(pid) => pid,
                Err(e) => {
                    eprintln!("Invalid pipeline ID '{id}': {e}");
                    std::process::exit(1);
                }
            };
            let store_path = cwd.join(".archon").join("pipelines");
            let store = std::sync::Arc::new(archon_pipeline::PipelineStateStore::new(&store_path));
            let registry = std::sync::Arc::new(archon_core::agents::AgentRegistry::load(&cwd));
            let task_service: std::sync::Arc<dyn archon_core::tasks::TaskService> =
                std::sync::Arc::new(archon_core::tasks::DefaultTaskService::new(registry, 10000));
            let engine = archon_pipeline::DefaultPipelineEngine::new(store, task_service);
            use archon_pipeline::PipelineEngine;
            match engine.cancel(pipeline_id).await {
                Ok(()) => println!("Pipeline {id} cancelled."),
                Err(e) => {
                    eprintln!("Failed to cancel pipeline {id}: {e}");
                    std::process::exit(1);
                }
            }
        }
    }
    Ok(())
}

async fn init_leann(cwd: &std::path::Path) -> Option<archon_pipeline::runner::LeannIntegration> {
    let db_path = cwd.join(".archon").join("leann.db");
    if let Some(parent) = db_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match archon_leann::CodeIndex::new(&db_path, Default::default()) {
        Ok(idx) => {
            let li = archon_pipeline::runner::LeannIntegration::new(std::sync::Arc::new(idx));
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

/// Build a [`LeannSearcher`] for the research pipeline facade.
///
/// Mirrors `init_leann` but returns the trait object the research facade
/// expects instead of the coding-pipeline `LeannIntegration` wrapper.
/// Falls back to `None` when the LEANN index cannot be created, same as
/// the coding pipeline.
async fn init_research_leann(cwd: &std::path::Path) -> Option<Arc<dyn LeannSearcher>> {
    let db_path = cwd.join(".archon").join("leann.db");
    if let Some(parent) = db_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match archon_leann::CodeIndex::new(&db_path, Default::default()) {
        Ok(idx) => {
            let idx = std::sync::Arc::new(idx);
            // Best-effort indexing — failures are logged, not fatal.
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

fn detect_format(
    file: &std::path::Path,
    format: Option<&str>,
    src: &str,
) -> std::result::Result<archon_pipeline::PipelineFormat, anyhow::Error> {
    match format.as_ref().map(|s| s.as_ref()) {
        Some("yaml" | "yml") => Ok(archon_pipeline::PipelineFormat::Yaml),
        Some("json") => Ok(archon_pipeline::PipelineFormat::Json),
        Some(other) => {
            eprintln!("Unknown format: {other} (expected yaml or json)");
            std::process::exit(3);
        }
        None => match file.extension().and_then(|e| e.to_str()) {
            Some("json") => Ok(archon_pipeline::PipelineFormat::Json),
            Some("yaml" | "yml") => Ok(archon_pipeline::PipelineFormat::Yaml),
            _ => {
                if src.trim_start().starts_with('{') || src.trim_start().starts_with('[') {
                    Ok(archon_pipeline::PipelineFormat::Json)
                } else {
                    Ok(archon_pipeline::PipelineFormat::Yaml)
                }
            }
        },
    }
}

async fn poll_pipeline_status<E: archon_pipeline::PipelineEngine>(
    engine: E,
    id: archon_pipeline::PipelineId,
) {
    loop {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        match engine.status(id).await {
            Ok(run) => {
                let finished_count = run
                    .steps
                    .values()
                    .filter(|s| s.state == archon_pipeline::StepRunState::Finished)
                    .count();
                let total = run.steps.len();
                eprint!("\r[{}/{}] {:?}  ", finished_count, total, run.state);
                match run.state {
                    archon_pipeline::PipelineState::Finished => {
                        eprintln!();
                        break;
                    }
                    archon_pipeline::PipelineState::Failed => {
                        eprintln!();
                        std::process::exit(1);
                    }
                    archon_pipeline::PipelineState::Cancelled => {
                        eprintln!();
                        std::process::exit(2);
                    }
                    archon_pipeline::PipelineState::RolledBack => {
                        eprintln!();
                        std::process::exit(1);
                    }
                    _ => {}
                }
            }
            Err(e) => {
                eprintln!("\nFailed to get status: {e}");
                std::process::exit(1);
            }
        }
    }
}

fn print_pipeline_result(result: &archon_pipeline::runner::PipelineResult) {
    println!("\n=== Pipeline Complete ===");
    println!("Session: {}", result.session_id);
    println!("Agents run: {}", result.agent_results.len());
    println!("Total cost: ${:.4}", result.total_cost_usd);
    println!("Duration: {:.1}s", result.duration.as_secs_f64());
}

/// Build the GNN auto-trainer for pipeline-mode invocations.
///
/// Reference:
/// - `archon-pipeline/src/learning/gnn/auto_trainer_runtime.rs`
/// - `src/session.rs` (interactive session uses the same construction pattern)
///
/// Returns `None` if any of: auto-trainer disabled in config, GNN disabled in
/// config, learning DB cannot be opened. Pipeline runs are typically shorter
/// than the configured throttle (default 1h), so the spawned loop may not fire
/// even one training run within a single pipeline invocation. The wiring is
/// here so:
///   1. Memory + correction events from the pipeline runner increment the
///      counters (visible across pipeline + interactive runs that share the
///      same learning DB)
///   2. The `LearningIntegration::on_memory_stored` / `on_correction_recorded`
///      hooks resolve to the live AutoTrainer Arc instead of `None`
///   3. /learning-status (run from interactive shell) sees consistent state
fn build_pipeline_auto_trainer(
    config: &ArchonConfig,
    cwd: &std::path::Path,
) -> Option<Arc<archon_pipeline::learning::gnn::auto_trainer::AutoTrainer>> {
    let at_cfg = &config.learning.gnn.auto_trainer;
    if !at_cfg.enabled || !config.learning.gnn.enabled {
        return None;
    }

    let db_path = cwd.join(".archon").join("learning.db");
    if let Some(parent) = db_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // "newrocksdb" engine = pure-Rust rust-rocksdb binding (cozo storage-new-rocksdb feature)
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
