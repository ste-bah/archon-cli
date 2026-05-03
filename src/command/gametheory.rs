//! CLI handler for `archon gametheory` commands.
//!
//! Subcommands: run, list-runs, show, inspect-routing, replay.

use std::sync::Arc;

use anyhow::Result;
use cozo::DbInstance;

use crate::cli_args::GametheoryAction;
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
    match action {
        GametheoryAction::Run {
            situation,
            classify_only,
            spec_path,
            debug_memory,
            budget,
            max_concurrent,
            style,
        } => {
            handle_run(
                situation,
                *classify_only,
                spec_path.as_deref(),
                *debug_memory,
                *budget,
                *max_concurrent,
                style,
                config,
                env_vars,
            )
            .await
        }
        GametheoryAction::ListRuns => handle_list_runs(),
        GametheoryAction::Show { run_id } => handle_show(run_id),
        GametheoryAction::InspectRouting { run_id } => handle_inspect_routing(run_id),
        GametheoryAction::Replay { run_id, spec_path } => {
            handle_replay(run_id, spec_path.as_deref(), config, env_vars)
        }
        GametheoryAction::Specimens { filter, ingest } => {
            handle_specimens(filter.as_deref(), *ingest)
        }
    }
}

/// Build an LLM client adapter from config. Returns None and logs a warning if auth fails.
fn build_llm_client(
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

fn open_memory_context(debug: bool) -> Result<gametheory::GameTheoryMemoryContext> {
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
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
) -> Result<()> {
    let llm = build_llm_client(config, env_vars);
    let llm_ref: Option<&dyn LlmClient> = llm.as_ref().map(|a| a as &dyn LlmClient);
    let path = spec_path.map(std::path::Path::new);
    let memory_ctx = open_memory_context(debug_memory)?;
    let options = gametheory::GameTheoryRunOptions {
        budget_usd: budget,
        max_concurrent,
        style_profile_id: Some(style.to_string()),
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
    gametheory::schema::ensure_gametheory_schema(&db)
        .map_err(|e| anyhow::anyhow!("schema init failed: {e}"))?;

    let result = db.run_script(
        "?[run_id, started_at, status] := *gt_runs{run_id, situation, started_at, completed_at, status}",
        Default::default(),
        cozo::ScriptMutability::Immutable,
    )
    .map_err(|e| anyhow::anyhow!("query gt_runs failed: {e}"))?;

    if result.rows.is_empty() {
        println!("No game-theory runs found.");
        return Ok(());
    }

    println!("Game-Theory Runs");
    println!("================");
    for row in &result.rows {
        let run_id = row[0].get_str().unwrap_or("?");
        let started = row[1].get_str().unwrap_or("?");
        let status = row[2].get_str().unwrap_or("?");
        println!("  {run_id}  {started}  {status}");
    }
    println!("{} run(s)", result.rows.len());
    Ok(())
}

// ── show ─────────────────────────────────────────────────────────────────────

fn handle_show(run_id: &str) -> Result<()> {
    let db = open_db()?;
    gametheory::schema::ensure_gametheory_schema(&db)
        .map_err(|e| anyhow::anyhow!("schema init failed: {e}"))?;

    // Query run info
    let runs = db.run_script(
        "?[situation, started_at, status] := *gt_runs{run_id, situation, started_at, completed_at, status}, run_id = $rid",
        {
            let mut p = std::collections::BTreeMap::new();
            p.insert("rid".into(), cozo::DataValue::from(run_id));
            p
        },
        cozo::ScriptMutability::Immutable,
    )
    .map_err(|e| anyhow::anyhow!("query gt_runs failed: {e}"))?;

    if runs.rows.is_empty() {
        println!("Run '{run_id}' not found.");
        return Ok(());
    }

    let situation = runs.rows[0][0].get_str().unwrap_or("?");
    let started = runs.rows[0][1].get_str().unwrap_or("?");
    let status = runs.rows[0][2].get_str().unwrap_or("?");

    println!("Run: {run_id}");
    println!("  Situation:  {situation}");
    println!("  Started:    {started}");
    println!("  Status:     {status}");

    // Query fingerprint
    let fps = db.run_script(
        "?[primary_family, created_at] := *gt_fingerprints{run_id, fingerprint_json, primary_family, created_at}, run_id = $rid",
        {
            let mut p = std::collections::BTreeMap::new();
            p.insert("rid".into(), cozo::DataValue::from(run_id));
            p
        },
        cozo::ScriptMutability::Immutable,
    )
    .map_err(|e| anyhow::anyhow!("query gt_fingerprints failed: {e}"))?;

    if !fps.rows.is_empty() {
        println!("  Family:     {}", fps.rows[0][0].get_str().unwrap_or("?"));
    }

    // Query report if available
    let reports = db.run_script(
        "?[word_count, created_at] := *gt_final_reports{run_id, report_md, created_at, total_cost_usd, total_duration_ms}, run_id = $rid",
        {
            let mut p = std::collections::BTreeMap::new();
            p.insert("rid".into(), cozo::DataValue::from(run_id));
            p
        },
        cozo::ScriptMutability::Immutable,
    )
    .map_err(|e| anyhow::anyhow!("query gt_final_reports failed: {e}"))?;

    if !reports.rows.is_empty() {
        let word_count = reports.rows[0][0].get_str().unwrap_or("?");
        println!("  Report:     {word_count} words");
    }

    println!();
    Ok(())
}

// ── inspect-routing ──────────────────────────────────────────────────────────

fn handle_inspect_routing(run_id: &str) -> Result<()> {
    let db = open_db()?;
    gametheory::schema::ensure_gametheory_schema(&db)
        .map_err(|e| anyhow::anyhow!("schema init failed: {e}"))?;

    let result = db
        .run_script(
            "?[enabled_specialists_json, skipped_specialists_json, evaluated_conditions_json] \
         := *gt_routing_decisions{run_id, fingerprint_id, enabled_specialists_json, \
         skipped_specialists_json, evaluated_conditions_json, created_at}, run_id = $rid",
            {
                let mut p = std::collections::BTreeMap::new();
                p.insert("rid".into(), cozo::DataValue::from(run_id));
                p
            },
            cozo::ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("query gt_routing_decisions failed: {e}"))?;

    if result.rows.is_empty() {
        println!("No routing decision found for run '{run_id}'.");
        return Ok(());
    }

    let enabled_json = result.rows[0][0].get_str().unwrap_or("[]");
    let skipped_json = result.rows[0][1].get_str().unwrap_or("[]");
    let conditions_json = result.rows[0][2].get_str().unwrap_or("[]");

    let enabled: Vec<String> = serde_json::from_str(enabled_json).unwrap_or_default();
    let skipped: Vec<(String, String)> = serde_json::from_str(skipped_json).unwrap_or_default();
    let conditions: Vec<(String, bool)> = serde_json::from_str(conditions_json).unwrap_or_default();

    println!("Routing Decision for {run_id}");
    println!("==============================");
    println!();
    println!("Enabled Specialists ({}):", enabled.len());
    for agent in &enabled {
        println!("  - {agent}");
    }
    println!();
    if !skipped.is_empty() {
        println!("Skipped Specialists ({}):", skipped.len());
        for (agent, reason) in &skipped {
            println!("  - {agent}: {reason}");
        }
        println!();
    }
    if !conditions.is_empty() {
        println!("Evaluated Conditions ({}):", conditions.len());
        for (expr, result) in &conditions {
            println!("  [{result}] {expr}");
        }
        println!();
    }

    Ok(())
}

// ── replay ───────────────────────────────────────────────────────────────────

fn handle_replay(
    run_id: &str,
    spec_path: Option<&str>,
    _config: &ArchonConfig,
    _env_vars: &ArchonEnvVars,
) -> Result<()> {
    let db = open_db()?;
    gametheory::schema::ensure_gametheory_schema(&db)
        .map_err(|e| anyhow::anyhow!("schema init failed: {e}"))?;

    // Load fingerprint
    let fps = db.run_script(
        "?[fingerprint_json] := *gt_fingerprints{run_id, fingerprint_json, primary_family, created_at}, run_id = $rid",
        {
            let mut p = std::collections::BTreeMap::new();
            p.insert("rid".into(), cozo::DataValue::from(run_id));
            p
        },
        cozo::ScriptMutability::Immutable,
    )
    .map_err(|e| anyhow::anyhow!("query gt_fingerprints failed: {e}"))?;

    if fps.rows.is_empty() {
        println!("Run '{run_id}' not found.");
        return Ok(());
    }

    let fp_json = fps.rows[0][0].get_str().unwrap_or("");
    let fingerprint: gametheory::GameTheoryFingerprint = serde_json::from_str(fp_json)
        .map_err(|e| anyhow::anyhow!("failed to parse fingerprint: {e}"))?;

    // Resolve spec path (same resolution as run_full_pipeline)
    let resolved = gametheory::resolve_spec_path(spec_path.map(std::path::Path::new))
        .map_err(|e| anyhow::anyhow!("failed to resolve spec path: {e}"))?;
    let spec = gametheory::load_spec(&resolved)
        .map_err(|e| anyhow::anyhow!("failed to load spec: {e}"))?;

    let rd = gametheory::evaluate_routing(&spec, &fingerprint, run_id, &fingerprint.created_at)
        .map_err(|e| anyhow::anyhow!("routing evaluation failed: {e}"))?;

    println!("Replay Routing for {run_id}");
    println!("============================");
    println!("Spec:        {}", resolved.display());
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

fn open_db() -> Result<DbInstance> {
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
}
