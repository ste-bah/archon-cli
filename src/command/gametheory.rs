//! CLI handler for `archon gametheory` commands.
//!
//! Subcommands: run, list-runs, show, status, inspect, inspect-routing, replay.

use std::sync::Arc;

use anyhow::Result;
use cozo::DbInstance;

use crate::cli_args::GametheoryAction;
use crate::command::gametheory_inspect;
use crate::command::pipeline_support::build_pipeline_learning_stack;
use crate::command::provider_gate::ensure_active_provider_supports;
use crate::runtime::llm::build_configured_llm_provider;
use archon_core::config::ArchonConfig;
use archon_core::env_vars::ArchonEnvVars;
#[cfg(test)]
use archon_llm::auth::AuthProvider;
#[cfg(test)]
use archon_llm::identity::IdentityMode;
#[cfg(test)]
use archon_llm::identity::{IdentityProvider, resolve_identity_mode};
use archon_pipeline::gametheory;
use archon_pipeline::llm_adapter::ProviderLlmAdapter;
use archon_pipeline::runner::LlmClient;

/// Dispatch the gametheory subcommand.
pub async fn handle_gametheory(
    action: Option<&GametheoryAction>,
    shorthand_situation: Option<&str>,
    shorthand_classify_only: bool,
    shorthand_kb: Option<&str>,
    shorthand_spec_path: Option<&str>,
    shorthand_debug_memory: bool,
    shorthand_budget: f64,
    shorthand_max_concurrent: usize,
    shorthand_style: &str,
    shorthand_enable_tier11: bool,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
) -> Result<()> {
    let replay_mutates = matches!(
        action,
        Some(GametheoryAction::Replay {
            reclassify: true,
            ..
        }) | Some(GametheoryAction::Replay {
            rerun_specialist: Some(_),
            ..
        })
    );
    if replay_mutates
        || matches!(
            action,
            Some(GametheoryAction::Run { .. } | GametheoryAction::Resume { .. })
        )
        || (action.is_none() && shorthand_situation.is_some())
    {
        preflight_run(&std::env::current_dir()?)?;
    }
    maybe_print_resume_hint(action)?;
    match action {
        Some(GametheoryAction::Run {
            situation,
            classify_only,
            spec_path,
            kb,
            debug_memory,
            budget,
            max_concurrent,
            style,
            enable_tier11,
        }) => {
            handle_run(
                &std::env::current_dir()?,
                situation,
                *classify_only,
                spec_path.as_deref(),
                kb.as_deref(),
                *debug_memory,
                *budget,
                *max_concurrent,
                style,
                *enable_tier11,
                config,
                env_vars,
            )
            .await
        }
        Some(GametheoryAction::ListRuns) => handle_list_runs(),
        Some(GametheoryAction::Show { run_id }) => handle_show(run_id),
        Some(GametheoryAction::Status { run_id }) => handle_status(run_id.as_deref()),
        Some(GametheoryAction::Inspect { artifact_id }) => handle_inspect(artifact_id),
        Some(GametheoryAction::InspectFingerprint { run_id }) => handle_inspect_fingerprint(run_id),
        Some(GametheoryAction::InspectRouting { run_id }) => handle_inspect_routing(run_id),
        Some(GametheoryAction::Replay {
            run_id,
            spec_path,
            reclassify,
            rerun_specialist,
        }) => {
            handle_replay(
                run_id,
                spec_path.as_deref(),
                *reclassify,
                rerun_specialist.as_deref(),
                config,
                env_vars,
            )
            .await
        }
        Some(GametheoryAction::Resume { run_id, spec_path }) => {
            handle_resume(run_id, spec_path.as_deref(), config, env_vars).await
        }
        Some(GametheoryAction::ListAgents { tier }) => handle_list_agents(*tier),
        Some(GametheoryAction::Specimens { filter, ingest }) => {
            handle_specimens(filter.as_deref(), *ingest)
        }
        None => {
            let situation = shorthand_situation.ok_or_else(|| {
                anyhow::anyhow!(
                    "Usage: archon gametheory \"<situation>\" [--kb <pack>] or archon gametheory run \"<situation>\""
                )
            })?;
            handle_run(
                &std::env::current_dir()?,
                situation,
                shorthand_classify_only,
                shorthand_spec_path,
                shorthand_kb,
                shorthand_debug_memory,
                shorthand_budget,
                shorthand_max_concurrent,
                shorthand_style,
                shorthand_enable_tier11,
                config,
                env_vars,
            )
            .await
        }
    }
}

fn maybe_print_resume_hint(action: Option<&GametheoryAction>) -> Result<()> {
    if matches!(action, Some(GametheoryAction::Resume { .. })) {
        return Ok(());
    }
    let db = open_db()?;
    let runs = gametheory::list_in_progress_runs(&db)
        .map_err(|e| anyhow::anyhow!("failed to scan in-progress gametheory runs: {e}"))?;
    if let Some(run) = runs.first() {
        eprintln!(
            "Resume available: archon gametheory resume {}  (started {}, situation: {})",
            run.run_id,
            run.started_at,
            truncate(&run.situation, 80)
        );
    }
    Ok(())
}

/// Build an LLM client adapter from config. Returns None and logs a warning if auth fails.
pub(crate) async fn build_llm_client(
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
) -> Option<ProviderLlmAdapter> {
    match build_configured_llm_provider(config, env_vars, "gametheory").await {
        Ok(provider) => Some(
            ProviderLlmAdapter::new(provider)
                .with_tier_aliases(config.llm.tier_aliases.clone())
                .with_model_prices(config.llm.model_prices.clone())
                .with_origin("gametheory"),
        ),
        Err(e) => {
            tracing::warn!("LLM auth unavailable for gametheory: {e}.");
            None
        }
    }
}

pub(crate) fn open_memory_context(debug: bool) -> Result<gametheory::GameTheoryMemoryContext> {
    let memory = archon_memory::MemoryGraph::open_default()
        .map_err(|e| anyhow::anyhow!("failed to open archon memory graph: {e}"))?;
    Ok(gametheory::GameTheoryMemoryContext::new(
        Arc::new(memory),
        None,
        debug,
    ))
}

// ── run ──────────────────────────────────────────────────────────────────────

async fn handle_run(
    cwd: &std::path::Path,
    situation: &str,
    classify_only: bool,
    spec_path: Option<&str>,
    kb: Option<&str>,
    debug_memory: bool,
    budget: f64,
    max_concurrent: usize,
    style: &str,
    enable_tier11: bool,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
) -> Result<()> {
    preflight_run(cwd)?;
    let db = open_db_at(cwd)?;
    if classify_only {
        run_classify_only(&db, situation, config, env_vars).await
    } else {
        ensure_active_provider_supports(
            config,
            archon_llm::providers::ProviderCapability::PipelineGametheory,
            "archon gametheory run",
        )?;
        run_full(
            &db,
            situation,
            spec_path,
            kb,
            debug_memory,
            budget,
            max_concurrent,
            style,
            enable_tier11,
            config,
            env_vars,
        )
        .await
    }
}

pub(crate) fn preflight_run(cwd: &std::path::Path) -> Result<()> {
    crate::command::pipeline_agents::preflight(
        cwd,
        archon_pipeline::runner::PipelineType::GameTheory,
    )
}

async fn run_classify_only(
    db: &DbInstance,
    situation: &str,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
) -> Result<()> {
    let llm = build_llm_client(config, env_vars).await;
    let llm_ref: Option<&dyn LlmClient> = llm.as_ref().map(|a| a as &dyn LlmClient);
    let cwd = std::env::current_dir()?;
    let (mut learning, _) = build_pipeline_learning_stack(config, &cwd);

    match gametheory::classify_with_learning(db, situation, llm_ref, learning.as_mut(), &cwd).await
    {
        Ok(fp) => {
            print_fingerprint(&fp);
            println!("Fingerprint persisted to Cozo (gt_runs, gt_fingerprints).");
            if llm.is_none() {
                println!(
                    "NOTE: LLM client unavailable; classify-only used the labelled keyword fallback. Set ANTHROPIC_API_KEY for real agent execution."
                );
            } else {
                println!("NOTE: real LLM-backed classification.");
            }
            Ok(())
        }
        Err(gametheory::GameTheoryError::EmptySituation) => {
            println!("Error: an empty situation is not valid.");
            println!("Usage: archon gametheory run \"<situation description>\"");
            Ok(())
        }
        Err(e) => anyhow::bail!("gametheory classification failed: {e}"),
    }
}

async fn run_full(
    db: &DbInstance,
    situation: &str,
    spec_path: Option<&str>,
    kb: Option<&str>,
    debug_memory: bool,
    budget: f64,
    max_concurrent: usize,
    style: &str,
    enable_tier11: bool,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
) -> Result<()> {
    let llm = build_llm_client(config, env_vars).await;
    let llm_ref: Option<&dyn LlmClient> = llm.as_ref().map(|a| a as &dyn LlmClient);
    let path = spec_path.map(std::path::Path::new);
    let memory_ctx = open_memory_context(debug_memory)?;
    let cwd = std::env::current_dir()?;
    let (mut learning, _) = build_pipeline_learning_stack(config, &cwd);
    let tier11_allowed = resolve_tier11_policy(enable_tier11)?;
    let options = gametheory::GameTheoryRunOptions {
        project_root: cwd.clone(),
        budget_usd: budget,
        max_concurrent,
        style_profile_id: Some(style.to_string()),
        enable_tier11: tier11_allowed,
        kb_pack_id: kb.map(str::to_string),
    };

    match gametheory::run_full_pipeline_with_learning_options(
        db,
        situation,
        path,
        llm_ref,
        memory_ctx,
        options,
        learning.as_mut(),
    )
    .await
    {
        Ok(result) => {
            println!("Game-Theory Strategic Analysis — Full Pipeline");
            println!("==============================================");
            println!("Run ID:            {}", result.run_id);
            println!("Status:            {}", result.status);
            println!("Primary Family:    {}", result.fingerprint.primary_family);
            println!(
                "Enabled Specialists: {}",
                result.routing_decision.enabled_specialists.len()
            );
            println!(
                "Skipped Specialists: {}",
                result.routing_decision.skipped_specialists.len()
            );
            println!("Specialist Count:  {}", result.specialist_count);
            println!(
                "Estimated Cost:    {}",
                specialist_cost_label(result.total_cost_usd, result.unknown_cost_count)
            );
            println!("Budget Cap:        ${budget:.2}");
            if let Some(kb) = kb {
                println!("Knowledge Pack:    {kb}");
            }
            println!("Max Concurrent:    {max_concurrent}");
            println!("Observed Concurrent: {}", result.max_observed_concurrent);
            println!("Style:             {style}");
            println!(
                "Tier 11:           {}",
                if tier11_allowed {
                    "enabled"
                } else {
                    "disabled"
                }
            );
            println!(
                "Report Length:     {} words",
                result.report.split_whitespace().count()
            );
            println!();

            if !result.failed_specialists.is_empty() {
                println!("Failed Specialists:");
                for (key, err) in &result.failed_specialists {
                    println!("  - {key}: {err}");
                }
                println!();
            }

            if !result.routing_decision.skipped_specialists.is_empty() {
                println!("Skipped:");
                for (key, reason) in &result.routing_decision.skipped_specialists {
                    println!("  - {key}: {reason}");
                }
                println!();
            }

            if debug_memory {
                println!("Memory Recall:");
                for audit in &result.memory_recall {
                    println!(
                        "  - {}: keys={}, cozo_hits={}, leann_hits={}",
                        audit.agent_key,
                        audit.memory_keys.len(),
                        audit.cozo_hits,
                        audit.leann_hits
                    );
                }
                if result.memory_recall.is_empty() {
                    println!("  - no real LLM memory recall performed");
                }
                println!();
            }

            if result.status == "BudgetExceeded" {
                println!("NOTE: budget cap halted specialist execution.");
            } else {
                println!("NOTE: real LLM-backed specialist execution.");
            }
            println!();
            println!(
                "Report persisted to Cozo (gt_runs, gt_routing_decisions, gt_specialist_outputs, gt_sections, gt_final_reports)."
            );
            Ok(())
        }
        Err(e) => anyhow::bail!("full pipeline failed: {e}"),
    }
}

// ── list-runs ────────────────────────────────────────────────────────────────

include!("gametheory_run_commands.rs");
include!("gametheory_output.rs");
