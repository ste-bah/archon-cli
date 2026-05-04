//! CLI handler for `archon gametheory` commands.
//!
//! Subcommands: run, list-runs, show, status, inspect, inspect-routing, replay.

use std::sync::Arc;

use anyhow::Result;
use cozo::DbInstance;

use crate::cli_args::GametheoryAction;
use crate::command::gametheory_inspect;
use archon_core::config::ArchonConfig;
use archon_core::env_vars::ArchonEnvVars;
use archon_pipeline::gametheory;
use archon_pipeline::llm_adapter::AnthropicLlmAdapter;
use archon_pipeline::runner::LlmClient;

/// Dispatch the gametheory subcommand.
pub async fn handle_gametheory(
    action: &GametheoryAction,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
) -> Result<()> {
    maybe_print_resume_hint(action)?;
    match action {
        GametheoryAction::Run {
            situation,
            classify_only,
            spec_path,
            debug_memory,
            budget,
            max_concurrent,
            style,
            enable_tier11,
        } => {
            handle_run(
                situation,
                *classify_only,
                spec_path.as_deref(),
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
        GametheoryAction::ListRuns => handle_list_runs(),
        GametheoryAction::Show { run_id } => handle_show(run_id),
        GametheoryAction::Status { run_id } => handle_status(run_id.as_deref()),
        GametheoryAction::Inspect { artifact_id } => handle_inspect(artifact_id),
        GametheoryAction::InspectFingerprint { run_id } => handle_inspect_fingerprint(run_id),
        GametheoryAction::InspectRouting { run_id } => handle_inspect_routing(run_id),
        GametheoryAction::Replay {
            run_id,
            spec_path,
            reclassify,
            rerun_specialist,
        } => {
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
        GametheoryAction::Resume { run_id, spec_path } => {
            handle_resume(run_id, spec_path.as_deref(), config, env_vars).await
        }
        GametheoryAction::ListAgents { tier } => handle_list_agents(*tier),
        GametheoryAction::Specimens { filter, ingest } => {
            handle_specimens(filter.as_deref(), *ingest)
        }
    }
}

fn maybe_print_resume_hint(action: &GametheoryAction) -> Result<()> {
    if matches!(action, GametheoryAction::Resume { .. }) {
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
pub(crate) fn build_llm_client(
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
) -> Option<AnthropicLlmAdapter> {
    let auth = match archon_llm::auth::resolve_auth_with_keys(
        non_empty(env_vars.anthropic_api_key.as_deref()),
        non_empty(env_vars.archon_api_key.as_deref()),
        non_empty(env_vars.archon_oauth_token.as_deref()),
        non_empty(std::env::var("ANTHROPIC_AUTH_TOKEN").ok().as_deref()),
    ) {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!("LLM auth unavailable for gametheory: {e}. Using keyword fallback.");
            return None;
        }
    };

    let identity = archon_llm::identity::IdentityProvider::new(
        archon_llm::identity::IdentityMode::Clean,
        uuid::Uuid::new_v4().to_string(),
        "gametheory-device".to_string(),
        String::new(),
    );

    let api_url = std::env::var("ANTHROPIC_BASE_URL")
        .ok()
        .or_else(|| config.api.base_url.clone());

    let client = archon_llm::anthropic::AnthropicClient::new(auth, identity, api_url);
    Some(AnthropicLlmAdapter::new(Arc::new(client)))
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
    situation: &str,
    classify_only: bool,
    spec_path: Option<&str>,
    debug_memory: bool,
    budget: f64,
    max_concurrent: usize,
    style: &str,
    enable_tier11: bool,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
) -> Result<()> {
    let db = open_db()?;
    let _llm = build_llm_client(config, env_vars);
    if classify_only {
        run_classify_only(&db, situation, config, env_vars).await
    } else {
        run_full(
            &db,
            situation,
            spec_path,
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

async fn run_classify_only(
    db: &DbInstance,
    situation: &str,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
) -> Result<()> {
    let llm = build_llm_client(config, env_vars);
    let llm_ref: Option<&dyn LlmClient> = llm.as_ref().map(|a| a as &dyn LlmClient);

    match gametheory::classify(db, situation, llm_ref).await {
        Ok(fp) => {
            print_fingerprint(&fp);
            println!("Fingerprint persisted to Cozo (gt_runs, gt_fingerprints).");
            if llm.is_none() {
                println!(
                    "NOTE: LLM client unavailable, using keyword fallback. Set ANTHROPIC_API_KEY for real agent execution."
                );
            } else {
                println!("NOTE: real LLM agent execution (Phase 5).");
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
    debug_memory: bool,
    budget: f64,
    max_concurrent: usize,
    style: &str,
    enable_tier11: bool,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
) -> Result<()> {
    let llm = build_llm_client(config, env_vars);
    let llm_ref: Option<&dyn LlmClient> = llm.as_ref().map(|a| a as &dyn LlmClient);
    let path = spec_path.map(std::path::Path::new);
    let memory_ctx = open_memory_context(debug_memory)?;
    let tier11_allowed = resolve_tier11_policy(enable_tier11)?;
    let options = gametheory::GameTheoryRunOptions {
        budget_usd: budget,
        max_concurrent,
        style_profile_id: Some(style.to_string()),
        enable_tier11: tier11_allowed,
    };

    match gametheory::run_full_pipeline_with_options(
        db, situation, path, llm_ref, memory_ctx, options,
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
            println!("Estimated Cost:    ${:.6}", result.total_cost_usd);
            println!("Budget Cap:        ${budget:.2}");
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
            } else if llm.is_none() {
                println!(
                    "NOTE: LLM client unavailable, using keyword fallback. Set ANTHROPIC_API_KEY for real agent execution."
                );
            } else {
                println!("NOTE: real LLM agent execution (Phase 5).");
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

fn handle_list_runs() -> Result<()> {
    let db = open_db()?;
    print!("{}", gametheory_inspect::render_list_runs(&db)?);
    Ok(())
}

// ── show ─────────────────────────────────────────────────────────────────────

fn handle_show(run_id: &str) -> Result<()> {
    let db = open_db()?;
    print!("{}", gametheory_inspect::render_show(&db, run_id)?);
    Ok(())
}

fn handle_status(run_id: Option<&str>) -> Result<()> {
    let db = open_db()?;
    print!("{}", gametheory_inspect::render_status(&db, run_id)?);
    Ok(())
}

fn handle_inspect(artifact_id: &str) -> Result<()> {
    let db = open_db()?;
    print!(
        "{}",
        gametheory_inspect::render_inspect_artifact(&db, artifact_id)?
    );
    Ok(())
}

fn handle_inspect_fingerprint(run_id: &str) -> Result<()> {
    let db = open_db()?;
    print!(
        "{}",
        gametheory_inspect::render_inspect_fingerprint(&db, run_id)?
    );
    Ok(())
}

// ── inspect-routing ──────────────────────────────────────────────────────────

fn handle_inspect_routing(run_id: &str) -> Result<()> {
    let db = open_db()?;
    print!(
        "{}",
        gametheory_inspect::render_inspect_routing(&db, run_id)?
    );
    Ok(())
}

// ── replay ───────────────────────────────────────────────────────────────────

async fn handle_replay(
    run_id: &str,
    spec_path: Option<&str>,
    reclassify: bool,
    rerun_specialist: Option<&str>,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
) -> Result<()> {
    if reclassify && rerun_specialist.is_some() {
        anyhow::bail!("--reclassify and --rerun-specialist cannot be combined");
    }

    let db = open_db()?;

    if reclassify {
        let Some(situation) = gametheory_inspect::load_run_situation(&db, run_id)? else {
            println!("Run '{run_id}' not found.");
            return Ok(());
        };
        return run_full(
            &db,
            &situation,
            spec_path,
            false,
            20.0,
            4,
            "executive",
            false,
            config,
            env_vars,
        )
        .await;
    }

    if let Some(agent_key) = rerun_specialist {
        let llm = build_llm_client(config, env_vars);
        let llm_ref: Option<&dyn LlmClient> = llm.as_ref().map(|a| a as &dyn LlmClient);
        let result = gametheory::replay_single_specialist(
            &db,
            run_id,
            agent_key,
            llm_ref,
            open_memory_context(false)?,
            gametheory::GameTheoryRunOptions::default(),
        )
        .await
        .map_err(|e| anyhow::anyhow!("specialist replay failed: {e}"))?;

        println!("Replay Specialist for {run_id}");
        println!("==============================");
        println!("Agent:    {}", result.agent_key);
        println!("Status:   {}", result.status);
        println!("Cost USD: ${:.6}", result.cost_usd);
        println!("Output:   {}", result.output_summary);
        return Ok(());
    }

    let rd = gametheory::replay_routing_from_stored_fingerprint(
        &db,
        run_id,
        spec_path.map(std::path::Path::new),
    )
    .map_err(|e| anyhow::anyhow!("routing replay failed: {e}"))?;

    println!("Replay Routing for {run_id}");
    println!("============================");
    println!("Fingerprint: preserved from stored Tier 1 record");
    println!("Enabled: {}", rd.enabled_specialists.len());
    for agent in &rd.enabled_specialists {
        println!("  - {agent}");
    }
    if !rd.skipped_specialists.is_empty() {
        println!("Skipped: {}", rd.skipped_specialists.len());
        for (agent, reason) in &rd.skipped_specialists {
            println!("  - {agent}: {reason}");
        }
    }
    println!();
    Ok(())
}

async fn handle_resume(
    run_id: &str,
    spec_path: Option<&str>,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
) -> Result<()> {
    let db = open_db()?;
    let llm = build_llm_client(config, env_vars);
    let llm_ref: Option<&dyn LlmClient> = llm.as_ref().map(|a| a as &dyn LlmClient);
    let result = gametheory::resume_run_from_checkpoint(
        &db,
        run_id,
        spec_path.map(std::path::Path::new),
        llm_ref,
        open_memory_context(false)?,
        gametheory::GameTheoryRunOptions::default(),
    )
    .await
    .map_err(|e| anyhow::anyhow!("gametheory resume failed: {e}"))?;

    println!("Resume Run {}", result.run_id);
    println!("==============================");
    println!("Status:                     {}", result.status);
    println!("Resumed Specialists:        {}", result.resumed_specialists);
    println!("Failed Specialists:         {}", result.failed_specialists);
    println!(
        "Skipped Completed:         {}",
        result.skipped_completed_specialists
    );
    println!("Total Cost USD:             ${:.6}", result.total_cost_usd);
    println!("Report Length:              {} words", result.report_words);
    Ok(())
}

fn truncate(value: &str, max_chars: usize) -> String {
    let mut truncated: String = value.chars().take(max_chars).collect();
    if value.chars().count() > max_chars {
        truncated.push_str("...");
    }
    truncated
}

fn resolve_tier11_policy(requested: bool) -> Result<bool> {
    resolve_tier11_policy_for_workspace(requested, &std::env::current_dir()?)
}

fn resolve_tier11_policy_for_workspace(
    requested: bool,
    workspace_dir: &std::path::Path,
) -> Result<bool> {
    if !requested {
        return Ok(false);
    }
    let load = archon_policy::load_policy_for_workspace(workspace_dir)
        .map_err(|e| anyhow::anyhow!("failed to load policy: {e}"))?;
    let decision = load.policy.gametheory_tier11_decision();
    if decision.allowed {
        Ok(true)
    } else {
        anyhow::bail!("--enable-tier11 denied: {}", decision.reason)
    }
}

fn handle_list_agents(tier: Option<u8>) -> Result<()> {
    print!("{}", gametheory_inspect::render_list_agents(tier)?);
    Ok(())
}

// ── specimens ────────────────────────────────────────────────────────────────

fn handle_specimens(filter: Option<&str>, ingest: bool) -> Result<()> {
    let db = open_db()?;
    let load = gametheory::specimens::ensure_specimen_library_loaded(&db, ingest)
        .map_err(|e| anyhow::anyhow!("specimen ingest failed: {e}"))?;
    let records = gametheory::specimens::list_specimens(&db, filter)
        .map_err(|e| anyhow::anyhow!("specimen query failed: {e}"))?;

    println!("Game-Theory Specimen Library");
    println!("============================");
    println!("Rows:       {}", records.len());
    println!("Inserted:   {}", load.inserted);
    if let Some(filter) = filter {
        println!("Filter:     {filter}");
    }
    println!();

    for record in records {
        println!(
            "  {}  cooperation={} payoff_sum={} timing={} horizon={}",
            record.situation_type,
            record.cooperation,
            record.payoff_sum,
            record.timing,
            record.horizon
        );
    }
    Ok(())
}

// ── helpers ──────────────────────────────────────────────────────────────────

/// Filter empty strings to None so they don't shadow real credentials.
fn non_empty(s: Option<&str>) -> Option<&str> {
    match s {
        Some("") => None,
        other => other,
    }
}

fn print_fingerprint(fp: &gametheory::GameTheoryFingerprint) {
    println!("Game-Theory Fingerprint");
    println!("=======================");
    println!("Run ID:         {}", fp.run_id);
    println!("Primary Family: {}", fp.primary_family);
    if let Some(ref classic) = fp.nearest_classic {
        println!("Nearest Classic: {}", classic);
    }
    println!();
    println!("Axes:");
    println!(
        "  Cooperation:    {:20} ({})",
        fp.cooperation.value, fp.cooperation.confidence
    );
    println!(
        "  Payoff Sum:     {:20} ({})",
        fp.payoff_sum.value, fp.payoff_sum.confidence
    );
    println!(
        "  Symmetry:       {:20} ({})",
        fp.symmetry.value, fp.symmetry.confidence
    );
    println!(
        "  Timing:         {:20} ({})",
        fp.timing.value, fp.timing.confidence
    );
    println!(
        "  Perfect Info:   {:20} ({})",
        fp.perfect_info.value, fp.perfect_info.confidence
    );
    println!(
        "  Complete Info:  {:20} ({})",
        fp.complete_info.value, fp.complete_info.confidence
    );
    println!(
        "  Cardinality:    {:20} ({})",
        fp.cardinality.value, fp.cardinality.confidence
    );
    println!(
        "  Strategy Space: {:20} ({})",
        fp.strategy_space.value, fp.strategy_space.confidence
    );
    println!(
        "  Horizon:        {:20} ({})",
        fp.horizon.value, fp.horizon.confidence
    );

    if !fp.shadow_games.is_empty() {
        println!();
        println!("Shadow Games:");
        for sg in &fp.shadow_games {
            println!("  - {}", sg);
        }
    }

    if !fp.ambiguities.is_empty() {
        println!();
        println!("Ambiguities:");
        for a in &fp.ambiguities {
            println!("  - [{}] {}", a.axis, a.note);
        }
    }

    if let Some(ref hg) = fp.hidden_game_scan {
        println!();
        println!("Hidden Game Scan: {} ({})", hg.game_name, hg.confidence);
        println!("  {}", hg.description);
    }

    println!();
}

pub(crate) fn open_db() -> Result<DbInstance> {
    let data_dir = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from(".local/share"))
        .join("archon");
    std::fs::create_dir_all(&data_dir)?;
    let path = data_dir.join("archon-data.db");
    let path_str = path.to_string_lossy().to_string();
    DbInstance::new("sqlite", &path_str, "")
        .map_err(|e| anyhow::anyhow!("Failed to open gametheory store at {path_str}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_non_empty_filters_empty_string() {
        assert_eq!(non_empty(Some("")), None);
        assert_eq!(non_empty(Some("key")), Some("key"));
        assert_eq!(non_empty(None), None);
    }

    #[test]
    fn test_non_empty_preserves_valid_values() {
        assert_eq!(non_empty(Some("sk-ant-123")), Some("sk-ant-123"));
        assert_eq!(non_empty(Some(" ")), Some(" ")); // whitespace is NOT empty
    }

    #[test]
    fn test_tier11_requires_policy_approval() {
        let tmp = tempfile::tempdir().unwrap();
        let denied = resolve_tier11_policy_for_workspace(true, tmp.path()).unwrap_err();
        assert!(denied.to_string().contains("Tier 11 is disabled"));

        let policy_dir = tmp.path().join(".archon");
        std::fs::create_dir_all(&policy_dir).unwrap();
        std::fs::write(
            policy_dir.join("policy.toml"),
            "[policy.gametheory]\nenable_tier11 = true\n",
        )
        .unwrap();
        assert!(resolve_tier11_policy_for_workspace(true, tmp.path()).unwrap());
    }
}
