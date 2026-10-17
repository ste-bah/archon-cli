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
        Ok(provider) => Some(ProviderLlmAdapter::new(provider).with_origin("gametheory")),
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

    if reclassify || rerun_specialist.is_some() {
        preflight_run(&std::env::current_dir()?)?;
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
            None,
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
        let llm = build_llm_client(config, env_vars).await;
        let llm_ref: Option<&dyn LlmClient> = llm.as_ref().map(|a| a as &dyn LlmClient);
        let cwd = std::env::current_dir()?;
        let (mut learning, _) = build_pipeline_learning_stack(config, &cwd);
        let result = gametheory::replay_single_specialist_with_learning(
            &db,
            run_id,
            agent_key,
            llm_ref,
            open_memory_context(false)?,
            gametheory::GameTheoryRunOptions {
                project_root: cwd,
                ..gametheory::GameTheoryRunOptions::default()
            },
            learning.as_mut(),
        )
        .await
        .map_err(|e| anyhow::anyhow!("specialist replay failed: {e}"))?;

        println!("Replay Specialist for {run_id}");
        println!("==============================");
        println!("Agent:    {}", result.agent_key);
        println!("Status:   {}", result.status);
        println!(
            "Cost USD: {}",
            result
                .cost_usd
                .map(|value| format!("${value:.6}"))
                .unwrap_or_else(|| "unknown".to_string())
        );
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
    handle_resume_with_cwd(
        &std::env::current_dir()?,
        run_id,
        spec_path,
        config,
        env_vars,
    )
    .await
}

async fn handle_resume_with_cwd(
    cwd: &std::path::Path,
    run_id: &str,
    spec_path: Option<&str>,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
) -> Result<()> {
    crate::command::pipeline_agents::preflight(
        cwd,
        archon_pipeline::runner::PipelineType::GameTheory,
    )?;
    let db = open_db()?;
    let llm = build_llm_client(config, env_vars).await;
    let llm_ref: Option<&dyn LlmClient> = llm.as_ref().map(|a| a as &dyn LlmClient);
    let result = gametheory::resume_run_from_checkpoint(
        &db,
        run_id,
        spec_path.map(std::path::Path::new),
        llm_ref,
        open_memory_context(false)?,
        gametheory::GameTheoryRunOptions {
            project_root: cwd.to_path_buf(),
            ..gametheory::GameTheoryRunOptions::default()
        },
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
    println!(
        "Total Cost USD:             {}",
        specialist_cost_label(result.total_cost_usd, result.unknown_cost_count)
    );
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

fn specialist_cost_label(known: f64, unknown_specialists: usize) -> String {
    if unknown_specialists == 0 {
        format!("${known:.6}")
    } else {
        format!("at least ${known:.6} ({unknown_specialists} specialists unknown)")
    }
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
#[cfg(test)]
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

pub(crate) fn open_db() -> Result<Arc<DbInstance>> {
    crate::command::store_paths::open_evidence_db("gametheory", &["ARCHON_GAMETHEORY_DB_PATH"])
}

fn open_db_at(cwd: &std::path::Path) -> Result<Arc<DbInstance>> {
    let path =
        crate::command::store_paths::evidence_db_path_for_dir(cwd, &["ARCHON_GAMETHEORY_DB_PATH"]);
    crate::command::store_paths::open_sqlite_db(&path, "gametheory")
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

    #[tokio::test]
    async fn gametheory_run_preflight_failure_creates_no_project_files() {
        let tmp = tempfile::tempdir().unwrap();
        let config = ArchonConfig::default();
        let env_vars = archon_core::env_vars::load_env_vars_from(&std::collections::HashMap::new());
        let error = handle_run(
            tmp.path(),
            "test situation",
            false,
            None,
            None,
            false,
            1.0,
            1,
            "",
            false,
            &config,
            &env_vars,
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(error.contains("GameTheory"));
        assert!(error.contains("archon agent sync"));
        for key in archon_pipeline::required_builtin_agent_keys(
            archon_pipeline::runner::PipelineType::GameTheory,
        ) {
            assert!(
                error.contains(&key),
                "missing key {key} absent from {error}"
            );
        }
        assert!(!tmp.path().join(".archon/archon-data.db").exists());
        assert!(!tmp.path().join(".archon").exists());
    }

    #[tokio::test]
    async fn gametheory_run_rejects_frontmatter_only_before_database_or_state_creation() {
        let tmp = tempfile::tempdir().unwrap();
        let (key, relative) = archon_pipeline::required_builtin_agent_assets(
            archon_pipeline::runner::PipelineType::GameTheory,
        )
        .into_iter()
        .next()
        .unwrap();
        let asset = tmp.path().join(relative);
        std::fs::create_dir_all(asset.parent().unwrap()).unwrap();
        std::fs::write(&asset, "---\nname: test\n---\n").unwrap();
        let error = handle_run(
            tmp.path(),
            "test situation",
            false,
            None,
            None,
            false,
            1.0,
            1,
            "",
            false,
            &ArchonConfig::default(),
            &archon_core::env_vars::load_env_vars_from(&std::collections::HashMap::new()),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(error.contains(key), "{error}");
        assert!(error.contains(relative), "{error}");
        assert!(error.contains("GameTheory"), "{error}");
        assert!(error.contains("archon agent sync"), "{error}");
        assert!(!tmp.path().join(".archon/archon-data.db").exists());
        assert!(!tmp.path().join(".pipeline-state").exists());
    }

    #[tokio::test]
    async fn gametheory_resume_rejects_frontmatter_only_before_database_creation() {
        let tmp = tempfile::tempdir().unwrap();
        let (key, relative) = archon_pipeline::required_builtin_agent_assets(
            archon_pipeline::runner::PipelineType::GameTheory,
        )
        .into_iter()
        .next()
        .unwrap();
        let asset = tmp.path().join(relative);
        std::fs::create_dir_all(asset.parent().unwrap()).unwrap();
        std::fs::write(&asset, "---\nname: test\n---\n").unwrap();
        let error = handle_resume_with_cwd(
            tmp.path(),
            "missing-run",
            None,
            &ArchonConfig::default(),
            &archon_core::env_vars::load_env_vars_from(&std::collections::HashMap::new()),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(error.contains(key), "{error}");
        assert!(error.contains(relative), "{error}");
        assert!(error.contains("GameTheory"), "{error}");
        assert!(error.contains("archon agent sync"), "{error}");
        assert!(!tmp.path().join(".archon/archon-data.db").exists());
    }

    #[test]
    fn test_gametheory_oauth_auth_forces_spoof_identity() {
        let config = ArchonConfig::default();
        let auth = AuthProvider::BearerToken(archon_llm::types::Secret::new(
            "sk-ant-oat-test".to_string(),
        ));
        let mode = resolve_identity_mode(&auth, false, &config.identity.as_view());

        match mode {
            IdentityMode::Spoof { betas, .. } => {
                assert!(
                    betas.iter().any(|beta| beta == "oauth-2025-04-20"),
                    "spoof identity must include the OAuth beta"
                );
            }
            other => panic!("OAuth/Bearer auth must use spoof identity, got {other:?}"),
        }
    }

    #[test]
    fn test_gametheory_oauth_spoof_identity_emits_claude_code_headers() {
        let config = ArchonConfig::default();
        let auth = AuthProvider::BearerToken(archon_llm::types::Secret::new(
            "sk-ant-oat-test".to_string(),
        ));
        let mode = resolve_identity_mode(&auth, false, &config.identity.as_view());
        let identity = IdentityProvider::new(
            mode,
            "session-test".to_string(),
            "device-test".to_string(),
            "account-test".to_string(),
        );
        let headers = identity.request_headers("request-test");

        assert_eq!(headers.get("x-app").map(String::as_str), Some("cli"));
        assert!(
            headers
                .get("User-Agent")
                .is_some_and(|value| value.starts_with("claude-cli/")),
            "spoof identity must use the Claude Code user agent"
        );
        assert!(
            headers
                .get("anthropic-beta")
                .is_some_and(|value| value.contains("oauth-2025-04-20")),
            "spoof identity must send the OAuth beta"
        );
    }

    #[test]
    fn test_gametheory_api_key_respects_clean_identity_config() {
        let config = ArchonConfig::default();
        let auth = AuthProvider::ApiKey(archon_llm::types::Secret::new(
            "sk-ant-api03-test".to_string(),
        ));
        let mode = resolve_identity_mode(&auth, false, &config.identity.as_view());

        assert!(
            matches!(mode, IdentityMode::Clean),
            "API key auth should respect the default clean identity mode"
        );
    }

    #[test]
    fn test_gametheory_api_key_respects_spoof_identity_config() {
        let mut config = ArchonConfig::default();
        config.identity.mode = "spoof".to_string();
        config.identity.spoof_version = "9.9.9".to_string();
        config.identity.spoof_entrypoint = "cli".to_string();
        let auth = AuthProvider::ApiKey(archon_llm::types::Secret::new(
            "sk-ant-api03-test".to_string(),
        ));
        let mode = resolve_identity_mode(&auth, false, &config.identity.as_view());

        match mode {
            IdentityMode::Spoof {
                entrypoint, betas, ..
            } => {
                assert_eq!(entrypoint, "cli");
                assert!(
                    betas.iter().any(|beta| beta == "claude-code-20250219"),
                    "spoof identity must include the Claude Code identity beta"
                );
            }
            other => panic!("spoof config should produce spoof identity, got {other:?}"),
        }
    }
}
