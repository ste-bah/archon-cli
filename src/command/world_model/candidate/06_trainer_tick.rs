pub(super) fn render_trainer_tick(
    config: &archon_core::config::ArchonConfig,
    root: &Path,
    last_activity_age_ms: Option<u64>,
    last_training_age_ms: Option<u64>,
    battery_percent: Option<u8>,
    unplugged: bool,
) -> Result<String> {
    render_trainer_tick_controlled(
        config,
        root,
        last_activity_age_ms,
        last_training_age_ms,
        battery_percent,
        unplugged,
        None,
    )
}

pub(super) fn render_trainer_tick_controlled(
    config: &archon_core::config::ArchonConfig,
    root: &Path,
    last_activity_age_ms: Option<u64>,
    last_training_age_ms: Option<u64>,
    battery_percent: Option<u8>,
    unplugged: bool,
    should_stop: Option<&dyn Fn() -> bool>,
) -> Result<String> {
    render_trainer_tick_observed(
        config,
        root,
        last_activity_age_ms,
        last_training_age_ms,
        battery_percent,
        unplugged,
        should_stop,
        None,
    )
}

pub(super) fn render_trainer_tick_observed(
    config: &archon_core::config::ArchonConfig,
    root: &Path,
    last_activity_age_ms: Option<u64>,
    last_training_age_ms: Option<u64>,
    battery_percent: Option<u8>,
    unplugged: bool,
    should_stop: Option<&dyn Fn() -> bool>,
    progress: Option<&dyn Fn(&str, &str)>,
) -> Result<String> {
    if config.learning.world_model.jepa.enabled
        || config.learning.world_model.model_kind == JEPA_MODEL_KIND
    {
        return render_jepa_trainer_tick_observed(
            config,
            root,
            last_activity_age_ms,
            last_training_age_ms,
            battery_percent,
            unplugged,
            should_stop,
            progress,
        );
    }
    render_latent_trainer_tick(
        config,
        root,
        last_activity_age_ms,
        last_training_age_ms,
        battery_percent,
        unplugged,
        should_stop,
        progress,
    )
}

fn render_latent_trainer_tick(
    config: &archon_core::config::ArchonConfig,
    root: &Path,
    last_activity_age_ms: Option<u64>,
    last_training_age_ms: Option<u64>,
    battery_percent: Option<u8>,
    unplugged: bool,
    should_stop: Option<&dyn Fn() -> bool>,
    progress: Option<&dyn Fn(&str, &str)>,
) -> Result<String> {
    let backend = selected_training_backend(config);
    let auto = &config.learning.world_model.auto_trainer;
    let policy = archon_world_model::trainer::DynamicTrainerPolicy {
        min_throttle_ms: auto.min_throttle_ms,
        idle_required_ms: auto.idle_required_ms,
        battery_suspend_below_percent: auto.battery_suspend_below_percent,
        max_runtime_ms: auto.max_runtime_ms,
    };
    let trigger_policy = archon_world_model::trainer::DynamicTrainerTriggerPolicy {
        trigger_new_rows: auto.trigger_new_rows,
        trigger_surprises: auto.trigger_surprises,
        trigger_corrections: auto.trigger_corrections,
        trigger_elapsed_ms: auto.trigger_elapsed_ms,
        first_run_threshold: auto.first_run_threshold,
    };
    let store = WorldModelStore::open(root)?;
    let stats = store.cold_start_stats()?;
    let registry = ModelRegistry::open(root)?;
    let schedule =
        latent_training_schedule(root, stats.rows, registry.candidate_count()? as u64, last_training_age_ms)?;
    let adapter = build_embedding_adapter(config)?;
    emit_trainer_progress(progress, "latent_train_start", "running latent trainer tick");
    let run = archon_world_model::trainer::run_dynamic_training_once_controlled(
        root,
        config.learning.world_model.state_dim,
        backend.selected,
        config.learning.world_model.training.allow_cpu_fallback,
        adapter.as_ref(),
        policy,
        trigger_policy,
        archon_world_model::trainer::TrainerRuntimeSnapshot {
            last_activity_age_ms: last_activity_age_ms.unwrap_or(auto.idle_required_ms),
            last_training_age_ms: schedule.last_training_age_ms,
            battery_percent,
            unplugged,
        },
        archon_world_model::trainer::DynamicTrainerTriggerSnapshot {
            total_rows: stats.rows,
            candidate_count: schedule.candidate_count,
            new_rows_since_training: schedule.new_rows_since_training,
            surprises_since_training: 0,
            corrections_since_training: 0,
            elapsed_since_training_ms: schedule.last_training_age_ms,
        },
        should_stop,
    )?;
    emit_trainer_progress(progress, "latent_train_complete", "latent trainer tick finished");
    let auto_promotion = run
        .candidate_id
        .as_deref()
        .map(|id| render_auto_promote_transition(config, root, id))
        .unwrap_or_else(|| "none (no candidate)".into());

    Ok(format!(
        "World Model Trainer Tick\n\
         ========================\n\
         Decision: {:?}\n\
         Should train: {}\n\
         Backend: {}\n\
         Backend fallback: {}\n\
         Trigger: {}\n\
         Rows loaded: {}\n\
         Examples: {}\n\
         Candidate: {}\n\
         Mean cosine error: {}\n\
         Auto promotion: {}\n\
         Checkpoint: {}",
        run.decision.reason,
        run.decision.should_train,
        backend.selected,
        backend.fallback_reason.as_deref().unwrap_or("none"),
        run.trigger
            .as_ref()
            .map(|trigger| format!("{trigger:?}"))
            .unwrap_or_else(|| "none".into()),
        run.rows_loaded,
        run.examples,
        run.candidate_id.as_deref().unwrap_or("none"),
        run.training_mean_cosine_error
            .map(|value| format!("{value:.4}"))
            .unwrap_or_else(|| "none".into()),
        auto_promotion,
        run.checkpoint_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "none".into())
    ))
}

fn render_jepa_trainer_tick_observed(
    config: &archon_core::config::ArchonConfig,
    root: &Path,
    last_activity_age_ms: Option<u64>,
    last_training_age_ms: Option<u64>,
    battery_percent: Option<u8>,
    unplugged: bool,
    should_stop: Option<&dyn Fn() -> bool>,
    progress: Option<&dyn Fn(&str, &str)>,
) -> Result<String> {
    let auto = &config.learning.world_model.auto_trainer;
    let policy = archon_world_model::trainer::DynamicTrainerPolicy {
        min_throttle_ms: auto.min_throttle_ms,
        idle_required_ms: auto.idle_required_ms,
        battery_suspend_below_percent: auto.battery_suspend_below_percent,
        max_runtime_ms: config.learning.world_model.jepa.max_runtime_ms,
    };
    let trigger_policy = archon_world_model::trainer::DynamicTrainerTriggerPolicy {
        trigger_new_rows: auto.trigger_new_rows,
        trigger_surprises: auto.trigger_surprises,
        trigger_corrections: auto.trigger_corrections,
        trigger_elapsed_ms: auto.trigger_elapsed_ms,
        first_run_threshold: auto.first_run_threshold,
    };
    emit_trainer_progress(progress, "jepa_schedule", "checking JEPA training trigger");
    let stats = WorldModelStore::open(root)?.cold_start_stats()?;
    let schedule = jepa_training_schedule(
        root,
        stats.rows,
        jepa_candidate_count(root) as u64,
        last_training_age_ms,
    )?;
    let runtime = archon_world_model::trainer::TrainerRuntimeSnapshot {
        last_activity_age_ms: last_activity_age_ms.unwrap_or(auto.idle_required_ms),
        last_training_age_ms: schedule.last_training_age_ms,
        battery_percent,
        unplugged,
    };
    let mut decision = archon_world_model::trainer::evaluate_dynamic_trainer(policy, runtime);
    let trigger = archon_world_model::trainer::evaluate_trainer_trigger(
        trigger_policy,
        archon_world_model::trainer::DynamicTrainerTriggerSnapshot {
            total_rows: stats.rows,
            candidate_count: schedule.candidate_count,
            new_rows_since_training: schedule.new_rows_since_training,
            surprises_since_training: 0,
            corrections_since_training: 0,
            elapsed_since_training_ms: schedule.last_training_age_ms,
        },
    );
    if decision.should_train && trigger.is_none() {
        decision.should_train = false;
        decision.reason = archon_world_model::trainer::TrainerDecisionReason::NoTrigger;
    }

    let mut rows_loaded = 0usize;
    let mut examples = 0u64;
    let mut candidate_id: Option<String> = None;
    let mut loss_total: Option<f32> = None;
    let mut checkpoint_path: Option<std::path::PathBuf> = None;
    if decision.should_train {
        check_trainer_stop(should_stop, "jepa row load")?;
        emit_trainer_progress(progress, "jepa_row_load_start", "loading world-model rows");
        let rows = WorldModelStore::open(root)?.load_rows()?;
        rows_loaded = rows.len();
        emit_trainer_progress(
            progress,
            "jepa_row_load_complete",
            &format!("loaded {rows_loaded} row(s)"),
        );
        let jepa_config = jepa_training_config(config)?;
        let backend = selected_training_backend(config);
        let started = std::time::Instant::now();
        let (model, outcome) = archon_world_model::jepa::train_jepa_candidate_with_backend_observed(
            &rows,
            &jepa_config,
            backend.requested,
            config.learning.world_model.training.allow_cpu_fallback,
            should_stop,
            progress,
        )?;
        if started.elapsed().as_millis() > u128::from(policy.max_runtime_ms) {
            bail!("jepa world-model training exceeded max_runtime_ms");
        }
        check_trainer_stop(should_stop, "jepa candidate write")?;
        emit_trainer_progress(progress, "jepa_candidate_write_start", "writing candidate");
        let registry = ModelRegistry::open(root)?;
        let _ = registry.write_jepa_candidate(&model, &outcome)?;
        examples = model.metadata.example_count;
        candidate_id = Some(model.metadata.model_id.clone());
        loss_total = Some(outcome.losses.loss_total);
        checkpoint_path = Some(
            registry
                .load_jepa_candidate(&model.metadata.model_id)?
                .checkpoint
                .path,
        );
        emit_trainer_progress(progress, "jepa_candidate_write_complete", "candidate written");
    }
    let auto_promotion = candidate_id
        .as_deref()
        .map(|id| render_auto_promote_jepa(config, root, id))
        .unwrap_or_else(|| "none (no candidate)".into());

    Ok(format!(
        "World Model JEPA-Inspired Trainer Tick\n\
         =============================\n\
         Decision: {:?}\n\
         Should train: {}\n\
         Trigger: {}\n\
         Rows loaded: {}\n\
         Examples: {}\n\
         Candidate: {}\n\
         Loss total: {}\n\
         Auto promotion: {}\n\
         Checkpoint: {}",
        decision.reason,
        decision.should_train,
        trigger
            .as_ref()
            .map(|trigger| format!("{trigger:?}"))
            .unwrap_or_else(|| "none".into()),
        rows_loaded,
        examples,
        candidate_id.as_deref().unwrap_or("none"),
        loss_total
            .map(|value| format!("{value:.4}"))
            .unwrap_or_else(|| "none".into()),
        auto_promotion,
        checkpoint_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "none".into())
    ))
}

fn emit_trainer_progress(progress: Option<&dyn Fn(&str, &str)>, stage: &str, summary: &str) {
    if let Some(progress) = progress {
        progress(stage, summary);
    }
}

fn check_trainer_stop(should_stop: Option<&dyn Fn() -> bool>, stage: &str) -> Result<()> {
    if should_stop.is_some_and(|check| check()) {
        bail!("world-model trainer stopped or timed out during {stage}");
    }
    Ok(())
}
