use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use archon_core::config::ArchonConfig;
use archon_core::env_vars::ArchonEnvVars;
use archon_memory::{MemoryTrait, graph::MemoryGraph};
use archon_pipeline::audit::store::PipelineBundleStore;
use archon_pipeline::audit::types::{BundleStatus, PipelineEvent};
use chrono::Utc;

use crate::command::pipeline_support::{
    build_pipeline_adapter, build_pipeline_learning_stack, init_leann, init_research_leann,
    print_pipeline_result,
};
use crate::command::provider_gate::ensure_active_provider_supports;

pub(crate) async fn handle_status(cwd: &Path, session_id: &str) -> Result<()> {
    let bundle = PipelineBundleStore::new(cwd);
    if let Ok(state) = bundle.load_state(session_id) {
        println!("Session: {}", state.session_id);
        println!("Type: {:?}", state.pipeline_type);
        println!("Task: {}", state.task);
        println!("Status: {:?}", state.status);
        println!("Completed agents: {}", state.completed_agent_count);
        println!(
            "Total tokens: {} in / {} out",
            state.total_tokens_in, state.total_tokens_out
        );
        println!("Total cost: ${:.4}", state.total_cost_usd);
        println!("Started: {}", state.started_at);
        println!("Updated: {}", state.updated_at);
        if let Some(current) = &state.current_agent_key {
            println!("Current agent: {current}");
        }
        if let Some(error) = &state.last_error {
            println!("Last error: {error}");
        }
        return Ok(());
    }
    handle_legacy_status(cwd, session_id)
}

pub(crate) async fn handle_resume(
    cwd: &Path,
    session_id: &str,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
) -> Result<()> {
    let bundle = PipelineBundleStore::new(cwd);
    if let Ok(manifest) = bundle.load_manifest(session_id) {
        let adapter = build_pipeline_adapter(config, env_vars, "pipeline_resume").await?;
        match manifest.pipeline_type {
            archon_pipeline::runner::PipelineType::Coding => {
                ensure_active_provider_supports(
                    config,
                    archon_llm::providers::ProviderCapability::PipelineCoding,
                    "archon pipeline resume",
                )?;
                let (learning, _) = build_pipeline_learning_stack(config, cwd);
                let facade = archon_pipeline::coding::facade::CodingFacade::with_learning(learning)
                    .with_models(config.models.anthropic.clone())
                    .with_context(config.context.clone());
                let leann = init_leann(cwd).await;
                println!("Resuming audited coding pipeline...");
                let result = archon_pipeline::runner::resume_pipeline_audited(
                    &facade,
                    &adapter,
                    session_id,
                    cwd,
                    leann.as_ref(),
                    None,
                    None,
                )
                .await?;
                print_pipeline_result(&result, cwd).await;
                return Ok(());
            }
            archon_pipeline::runner::PipelineType::Research => {
                ensure_active_provider_supports(
                    config,
                    archon_llm::providers::ProviderCapability::PipelineResearch,
                    "archon pipeline resume",
                )?;
                let phd_learning =
                    archon_pipeline::learning::integration::PhDLearningIntegration::new();
                let memory: Arc<dyn MemoryTrait> = Arc::new(
                    MemoryGraph::in_memory().expect("in-memory graph for research resume"),
                );
                let leann = init_research_leann(cwd).await;
                let facade = archon_pipeline::research::facade::ResearchFacade::with_learning(
                    memory,
                    leann,
                    cwd.display().to_string(),
                    None,
                    phd_learning,
                )
                .with_models(config.models.anthropic.clone())
                .with_context(config.context.clone());
                println!("Resuming audited research pipeline...");
                let result = archon_pipeline::runner::resume_pipeline_audited(
                    &facade, &adapter, session_id, cwd, None, None, None,
                )
                .await?;
                print_pipeline_result(&result, cwd).await;
                return Ok(());
            }
            other => {
                eprintln!("Unsupported audited pipeline type for resume: {other:?}");
                std::process::exit(1);
            }
        }
    }
    handle_legacy_resume(cwd, session_id, config, env_vars).await
}

pub(crate) async fn handle_list(cwd: &Path) -> Result<()> {
    let bundle = PipelineBundleStore::new(cwd);
    match bundle.list_states() {
        Ok(states) if !states.is_empty() => {
            println!(
                "{:<38} {:<10} {:<10} {:>6} {:>10}  TASK",
                "SESSION ID", "TYPE", "STATUS", "AGENTS", "COST"
            );
            println!("{}", "-".repeat(100));
            for s in &states {
                let truncated: String = s.task.chars().take(30).collect();
                println!(
                    "{:<38} {:<10} {:<10} {:>6} ${:>9.4}  {}",
                    s.session_id,
                    format!("{:?}", s.pipeline_type),
                    format!("{:?}", s.status),
                    s.completed_agent_count,
                    s.total_cost_usd,
                    truncated,
                );
            }
            return Ok(());
        }
        Ok(_) => {}
        Err(e) => {
            eprintln!("Failed to list audited sessions: {e}");
            std::process::exit(1);
        }
    }
    handle_legacy_list(cwd)
}

pub(crate) async fn handle_abort(cwd: &Path, session_id: &str) -> Result<()> {
    let bundle = PipelineBundleStore::new(cwd);
    if let Ok(mut state) = bundle.load_state(session_id) {
        state.status = BundleStatus::Aborted;
        state.completed_at = Some(Utc::now());
        state.updated_at = Utc::now();
        state.last_error = Some("user aborted run".into());
        bundle.save_state(&state)?;
        bundle.append_event(
            session_id,
            PipelineEvent::RunAborted {
                reason: "user aborted run".into(),
            },
        )?;
        println!("Session {session_id} aborted.");
        return Ok(());
    }
    match archon_pipeline::session::abort(session_id, cwd) {
        Ok(()) => println!("Session {session_id} aborted."),
        Err(e) => {
            eprintln!("Failed to abort session {session_id}: {e}");
            std::process::exit(1);
        }
    }
    Ok(())
}

pub(crate) async fn handle_verify(cwd: &Path, session_id: &str, write_report: bool) -> Result<()> {
    let store = PipelineBundleStore::new(cwd);
    let report = archon_pipeline::audit::verify_bundle(&store, session_id, write_report)?;
    println!("Session: {}", report.session_id);
    println!("Valid: {}", report.valid);
    println!("Audit events: {}", report.audit_events);
    println!("Agent records: {}", report.agent_records);
    if !report.findings.is_empty() {
        println!("Findings:");
        for finding in &report.findings {
            println!("  [{}] {}", finding.severity, finding.message);
        }
    }
    if write_report {
        println!(
            "Report: {}",
            store
                .bundle_dir(session_id)
                .join("verification")
                .join("report.json")
                .display()
        );
    }
    if !report.valid {
        std::process::exit(2);
    }
    Ok(())
}

pub(crate) async fn handle_inspect(cwd: &Path, session_id: &str) -> Result<()> {
    let store = PipelineBundleStore::new(cwd);
    let manifest = store.load_manifest(session_id)?;
    let state = store.load_state(session_id)?;
    let agents = store.list_agent_records(session_id)?;
    println!("Session: {}", manifest.session_id);
    println!("Archon version: {}", manifest.archon_version);
    println!("Type: {:?}", manifest.pipeline_type);
    println!("Task: {}", manifest.task);
    println!("Created: {}", manifest.created_at);
    println!("Status: {:?}", state.status);
    println!("Completed agents: {}", state.completed_agent_count);
    println!("Bundle: {}", store.bundle_dir(session_id).display());
    println!("Agents:");
    for agent in &agents {
        println!(
            "  {:>3}. {:<32} phase {:<2} q={:.2} out={}",
            agent.ordinal,
            agent.agent_key,
            agent.phase,
            agent.quality.as_ref().map(|q| q.overall).unwrap_or(0.0),
            agent.output_hash,
        );
    }
    Ok(())
}

pub(crate) async fn handle_export_traces(
    cwd: &Path,
    session_id: &str,
    format: &str,
    out: Option<&PathBuf>,
    include_unverified: bool,
) -> Result<()> {
    if format != "jsonl" {
        eprintln!("Unsupported trace export format '{format}'. Supported: jsonl");
        std::process::exit(2);
    }
    let store = PipelineBundleStore::new(cwd);
    let jsonl = archon_pipeline::audit::export_jsonl(&store, session_id, include_unverified)?;
    if let Some(path) = out {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, jsonl)?;
        println!("{}", path.display());
    } else {
        print!("{jsonl}");
    }
    Ok(())
}

fn handle_legacy_status(cwd: &Path, session_id: &str) -> Result<()> {
    let cp_path = cwd
        .join(".pipeline-state")
        .join(session_id)
        .join("checkpoint.json");
    match std::fs::read_to_string(&cp_path) {
        Ok(data) => {
            let session: archon_pipeline::session::PipelineCheckpoint = serde_json::from_str(&data)
                .unwrap_or_else(|e| {
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
    Ok(())
}

async fn handle_legacy_resume(
    cwd: &Path,
    session_id: &str,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
) -> Result<()> {
    ensure_active_provider_supports(
        config,
        archon_llm::providers::ProviderCapability::PipelineCoding,
        "archon pipeline resume",
    )?;
    match archon_pipeline::session::resume(session_id, cwd) {
        Ok(session) => {
            println!("Resumed session: {}", session.session_id);
            println!("Completed agents: {}", session.completed_agents.len());
            let adapter = build_pipeline_adapter(config, env_vars, "pipeline_resume").await?;
            match format!("{:?}", &session.pipeline_type).as_str() {
                "Coding" => legacy_resume_coding(cwd, config, &adapter, &session.task).await?,
                "Research" => legacy_resume_research(cwd, config, &adapter, &session.task).await?,
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
    Ok(())
}

async fn legacy_resume_coding(
    cwd: &Path,
    config: &ArchonConfig,
    adapter: &archon_pipeline::llm_adapter::ProviderLlmAdapter,
    task: &str,
) -> Result<()> {
    let (learning, _) = build_pipeline_learning_stack(config, cwd);
    let facade = archon_pipeline::coding::facade::CodingFacade::with_learning(learning)
        .with_context(config.context.clone());
    println!("Resuming coding pipeline...");
    let result =
        archon_pipeline::runner::run_pipeline(&facade, adapter, task, None, None, None).await?;
    print_pipeline_result(&result, cwd).await;
    Ok(())
}

async fn legacy_resume_research(
    cwd: &Path,
    config: &ArchonConfig,
    adapter: &archon_pipeline::llm_adapter::ProviderLlmAdapter,
    task: &str,
) -> Result<()> {
    let phd_learning = archon_pipeline::learning::integration::PhDLearningIntegration::new();
    let memory: Arc<dyn MemoryTrait> =
        Arc::new(MemoryGraph::in_memory().expect("in-memory graph for research resume"));
    let leann = init_research_leann(cwd).await;
    let facade = archon_pipeline::research::facade::ResearchFacade::with_learning(
        memory,
        leann,
        cwd.display().to_string(),
        None,
        phd_learning,
    )
    .with_context(config.context.clone());
    println!("Resuming research pipeline...");
    let result =
        archon_pipeline::runner::run_pipeline(&facade, adapter, task, None, None, None).await?;
    print_pipeline_result(&result, cwd).await;
    Ok(())
}

fn handle_legacy_list(cwd: &Path) -> Result<()> {
    match archon_pipeline::session::list_sessions(cwd) {
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
    }
    Ok(())
}
