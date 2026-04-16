//! `/pipeline` slash command handler.
//! Extracted from main.rs to reduce main.rs from 1532 to < 500 lines.

use std::path::PathBuf;

use archon_core::cli_flags::ResolvedFlags;
use archon_core::config::ArchonConfig;
use archon_core::env_vars::ArchonEnvVars;
use archon_llm::auth::resolve_auth_with_keys;
use archon_llm::identity::{IdentityMode, IdentityProvider};

use crate::cli_args::PipelineAction;
use crate::setup;

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
                println!("  Phase 1: task-analyzer, requirement-extractor, requirement-prioritizer");
                println!("  Phase 2: pattern-explorer, technology-scout, feasibility-analyzer, codebase-analyzer");
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
                let pipe_client = archon_llm::anthropic::AnthropicClient::new(
                    pipe_auth,
                    identity,
                    api_url,
                );
                let adapter = archon_pipeline::llm_adapter::AnthropicLlmAdapter::new(std::sync::Arc::new(pipe_client));
                let learning = archon_pipeline::learning::integration::LearningIntegration::new(None, None, Default::default());
                let facade = archon_pipeline::coding::facade::CodingFacade::with_learning(learning);
                let leann = init_leann(&cwd).await;
                println!("Starting coding pipeline...");
                println!("Task: {task}");
                let result = archon_pipeline::runner::run_pipeline(
                    &facade,
                    &adapter,
                    task,
                    leann.as_ref(),
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
                let pipe_client = archon_llm::anthropic::AnthropicClient::new(
                    pipe_auth,
                    identity,
                    api_url,
                );
                let adapter = archon_pipeline::llm_adapter::AnthropicLlmAdapter::new(std::sync::Arc::new(pipe_client));
                let phd_learning = archon_pipeline::learning::integration::PhDLearningIntegration::new();
                let facade = archon_pipeline::research::facade::ResearchFacade::with_learning(None, phd_learning);
                println!("Starting research pipeline...");
                println!("Topic: {topic}");
                let result = archon_pipeline::runner::run_pipeline(&facade, &adapter, topic, None).await?;
                print_pipeline_result(&result);
            }
        }
        PipelineAction::Status { session_id } => {
            let cp_path = cwd.join(".pipeline-state").join(session_id).join("checkpoint.json");
            match std::fs::read_to_string(&cp_path) {
                Ok(data) => {
                    let session: archon_pipeline::session::PipelineCheckpoint = serde_json::from_str(&data).unwrap_or_else(|e| {
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
                        println!("  {} (quality: {:.2}, cost: ${:.4})", agent.agent_key, agent.quality_score, agent.cost_usd);
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
                    let pipe_client = archon_llm::anthropic::AnthropicClient::new(
                        pipe_auth,
                        identity,
                        api_url,
                    );
                    let adapter = archon_pipeline::llm_adapter::AnthropicLlmAdapter::new(std::sync::Arc::new(pipe_client));
                    let facade_type = &session.pipeline_type;
                    match format!("{:?}", facade_type).as_str() {
                        "Coding" => {
                            let learning = archon_pipeline::learning::integration::LearningIntegration::new(None, None, Default::default());
                            let facade = archon_pipeline::coding::facade::CodingFacade::with_learning(learning);
                            println!("Resuming coding pipeline...");
                            let result = archon_pipeline::runner::run_pipeline(
                                &facade,
                                &adapter,
                                &session.task,
                                None,
                            )
                            .await?;
                            print_pipeline_result(&result);
                        }
                        "Research" => {
                            let phd_learning = archon_pipeline::learning::integration::PhDLearningIntegration::new();
                            let facade = archon_pipeline::research::facade::ResearchFacade::with_learning(None, phd_learning);
                            println!("Resuming research pipeline...");
                            let result = archon_pipeline::runner::run_pipeline(
                                &facade,
                                &adapter,
                                &session.task,
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
        PipelineAction::List => {
            match archon_pipeline::session::list_sessions(&cwd) {
                Ok(sessions) if sessions.is_empty() => {
                    println!("No pipeline sessions found.");
                }
                Ok(sessions) => {
                    println!("{:<38} {:<10} {:<10} {:>6} {:>10}  {}", "SESSION ID", "TYPE", "STATUS", "AGENTS", "COST", "TASK");
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
            }
        }
        PipelineAction::Abort { session_id } => {
            match archon_pipeline::session::abort(session_id, &cwd) {
                Ok(()) => println!("Session {session_id} aborted."),
                Err(e) => {
                    eprintln!("Failed to abort session {session_id}: {e}");
                    std::process::exit(1);
                }
            }
        }
        PipelineAction::Run { file, format, detach } => {
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
        None => {
            match file.extension().and_then(|e| e.to_str()) {
                Some("json") => Ok(archon_pipeline::PipelineFormat::Json),
                Some("yaml" | "yml") => Ok(archon_pipeline::PipelineFormat::Yaml),
                _ => {
                    if src.trim_start().starts_with('{') || src.trim_start().starts_with('[') {
                        Ok(archon_pipeline::PipelineFormat::Json)
                    } else {
                        Ok(archon_pipeline::PipelineFormat::Yaml)
                    }
                }
            }
        }
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
                let finished_count = run.steps.values()
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
