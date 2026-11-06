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
