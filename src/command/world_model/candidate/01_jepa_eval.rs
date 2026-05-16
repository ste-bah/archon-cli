fn jepa_backend_forward_parity_passes(
    config: &archon_core::config::ArchonConfig,
    model: &archon_world_model::jepa::JepaTraceModel,
    rows: &[archon_world_model::schema::WorldTraceRow],
) -> bool {
    if matches!(
        model.metadata.backend,
        archon_world_model::BackendKind::Auto | archon_world_model::BackendKind::Cpu
    ) {
        return true;
    }
    let jepa_config = match jepa_training_config(config) {
        Ok(config) => config,
        Err(_) => return false,
    };
    let examples = match archon_world_model::jepa::build_jepa_training_examples(rows, &jepa_config)
    {
        Ok(examples) => examples,
        Err(_) => return false,
    };
    let Some(example) = examples.first() else {
        return false;
    };
    archon_world_model::jepa::jepa_backend_forward_parity_gate(
        model,
        &example.context,
        &example.action,
        config.learning.world_model.jepa.backend_parity_cosine_floor,
    )
}

pub(super) fn render_inspect_jepa(root: &Path, candidate_id: &str) -> Result<String> {
    let registry = ModelRegistry::open(root)?;
    let candidate = registry.load_jepa_candidate(candidate_id)?;
    let eval = registry.load_jepa_eval_report(candidate_id)?;

    Ok(format!(
        "World Model JEPA Inspect\n\
         ========================\n\
         Candidate: {candidate_id}\n\
         Model kind: {}\n\
         Latent dim: {}\n\
         Windows: context={} target={}\n\
         Horizons: {:?}\n\
         Mask ratio: {:.2}\n\
         EMA decay: {:.3}\n\
         Stop gradient: {}\n\
         Requested backend: {}\n\
         Selected backend: {}\n\
         Framework: {}\n\
         Device: {}\n\
         Fallback reason: {}\n\
         Native encode: {}\n\
         Native predictor fit: {}\n\
         Native auxiliary fit: {}\n\
         Native transition fit: {}\n\
         Native loss eval: {}\n\
         Native runtime prediction: {}\n\
         Host fallback count: {}\n\
         Hardware validation captured: {}\n\
         Validation examples: {}\n\
         Rows: {}\n\
         Examples: {}\n\
         Parameters: {}\n\
         Loss total: {:.4}\n\
         Loss improved: {}\n\
         Collapse gate: {} (std={:.4}, rank_ratio={:.4})\n\
         Horizon gate: {}\n\
         Last eval gates pass: {}\n\
         Checkpoint: {}\n\
         Training run: {}",
        candidate.model.metadata.model_kind,
        candidate.model.metadata.latent_dim,
        candidate.model.metadata.context_window_rows,
        candidate.model.metadata.target_window_rows,
        candidate.model.metadata.prediction_horizons,
        candidate.model.metadata.mask_ratio,
        candidate.model.metadata.ema_decay,
        candidate.model.metadata.target_stop_gradient,
        candidate.model.metadata.backend_execution.requested_backend,
        candidate.model.metadata.backend_execution.selected_backend,
        candidate.model.metadata.backend_execution.framework,
        candidate
            .model
            .metadata
            .backend_execution
            .device_name
            .as_deref()
            .unwrap_or("unknown"),
        candidate
            .model
            .metadata
            .backend_execution
            .fallback_reason
            .as_deref()
            .unwrap_or("none"),
        candidate.model.metadata.backend_execution.native_encode,
        candidate
            .model
            .metadata
            .backend_execution
            .native_predictor_fit,
        candidate
            .model
            .metadata
            .backend_execution
            .native_auxiliary_fit,
        candidate
            .model
            .metadata
            .backend_execution
            .native_transition_fit,
        candidate.model.metadata.backend_execution.native_loss_eval,
        candidate
            .model
            .metadata
            .backend_execution
            .native_runtime_prediction
            .unwrap_or(false),
        candidate
            .model
            .metadata
            .backend_execution
            .host_fallback_count,
        candidate
            .model
            .metadata
            .backend_execution
            .hardware_validation_captured_at
            .is_some(),
        candidate
            .model
            .metadata
            .backend_execution
            .validation_example_count,
        candidate.model.metadata.row_count,
        candidate.model.metadata.example_count,
        candidate.model.metadata.parameter_count,
        candidate.outcome.losses.loss_total,
        candidate.outcome.progress.improved,
        candidate.outcome.collapse.passes,
        candidate.outcome.collapse.mean_latent_std,
        candidate.outcome.collapse.effective_rank_ratio,
        candidate.outcome.horizon.passes,
        eval.as_ref()
            .map(|record| record.gates.passed)
            .unwrap_or(false),
        candidate.checkpoint.path.display(),
        candidate.training_run.display()
    ))
}

pub(super) fn render_compare_representations(
    config: &archon_core::config::ArchonConfig,
    root: &Path,
    baseline: &str,
    candidate_id: &str,
) -> Result<String> {
    let registry = ModelRegistry::open(root)?;
    let candidate = registry.load_jepa_candidate(candidate_id)?;
    let rows = WorldModelStore::open(root)?.load_rows()?;
    let report = compare_jepa_representations(config, &candidate.model, &rows, baseline, false);
    let path = registry.write_jepa_representation_comparison(&report)?;

    Ok(format!(
        "World Model Representation Comparison\n\
         =====================================\n\
         Candidate: {candidate_id}\n\
         Baseline: {}\n\
         Baseline available: {}\n\
         Heldout: {}\n\
         JEPA cosine similarity: {:.4}\n\
         Baseline cosine similarity: {:.4}\n\
         Relative improvement: {:.2}%\n\
         Brier regressed: {}\n\
         Passed: {}\n\
         Promotion baseline fixed: fastembed\n\
         Report: {}",
        report.baseline_backend,
        report.baseline_available,
        report.heldout_examples,
        report.jepa_next_state_cosine_similarity,
        report.baseline_next_state_cosine_similarity,
        report.relative_improvement * 100.0,
        report.brier_regressed,
        report.passed,
        path.display()
    ))
}

pub(super) fn render_trainer_tick(
    config: &archon_core::config::ArchonConfig,
    root: &Path,
    last_activity_age_ms: Option<u64>,
    last_training_age_ms: Option<u64>,
    battery_percent: Option<u8>,
    unplugged: bool,
) -> Result<String> {
    if config.learning.world_model.jepa.enabled
        || config.learning.world_model.model_kind == JEPA_MODEL_KIND
    {
        return render_jepa_trainer_tick(
            config,
            root,
            last_activity_age_ms,
            last_training_age_ms,
            battery_percent,
            unplugged,
        );
    }

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
    let candidate_count = registry.candidate_count()? as u64;
    let adapter = build_embedding_adapter(config)?;
    let run = archon_world_model::trainer::run_dynamic_training_once(
        root,
        config.learning.world_model.state_dim,
        backend.selected,
        config.learning.world_model.training.allow_cpu_fallback,
        adapter.as_ref(),
        policy,
        trigger_policy,
        archon_world_model::trainer::TrainerRuntimeSnapshot {
            last_activity_age_ms: last_activity_age_ms.unwrap_or(auto.idle_required_ms),
            last_training_age_ms,
            battery_percent,
            unplugged,
        },
        archon_world_model::trainer::DynamicTrainerTriggerSnapshot {
            total_rows: stats.rows,
            candidate_count,
            new_rows_since_training: if candidate_count == 0 { stats.rows } else { 0 },
            surprises_since_training: 0,
            corrections_since_training: 0,
            elapsed_since_training_ms: last_training_age_ms,
        },
    )?;

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
        run.checkpoint_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "none".into())
    ))
}

fn render_jepa_trainer_tick(
    config: &archon_core::config::ArchonConfig,
    root: &Path,
    last_activity_age_ms: Option<u64>,
    last_training_age_ms: Option<u64>,
    battery_percent: Option<u8>,
    unplugged: bool,
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
    let runtime = archon_world_model::trainer::TrainerRuntimeSnapshot {
        last_activity_age_ms: last_activity_age_ms.unwrap_or(auto.idle_required_ms),
        last_training_age_ms,
        battery_percent,
        unplugged,
    };
    let mut decision = archon_world_model::trainer::evaluate_dynamic_trainer(policy, runtime);
    let stats = WorldModelStore::open(root)?.cold_start_stats()?;
    let trigger = archon_world_model::trainer::evaluate_trainer_trigger(
        trigger_policy,
        archon_world_model::trainer::DynamicTrainerTriggerSnapshot {
            total_rows: stats.rows,
            candidate_count: jepa_candidate_count(root) as u64,
            new_rows_since_training: stats.rows,
            surprises_since_training: 0,
            corrections_since_training: 0,
            elapsed_since_training_ms: last_training_age_ms,
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
        let rows = WorldModelStore::open(root)?.load_rows()?;
        rows_loaded = rows.len();
        let jepa_config = jepa_training_config(config)?;
        let backend = selected_training_backend(config);
        let started = std::time::Instant::now();
        let (model, outcome) = archon_world_model::jepa::train_jepa_candidate_with_backend(
            &rows,
            &jepa_config,
            backend.requested,
            config.learning.world_model.training.allow_cpu_fallback,
        )?;
        if started.elapsed().as_millis() > u128::from(policy.max_runtime_ms) {
            bail!("jepa world-model training exceeded max_runtime_ms");
        }
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
    }

    Ok(format!(
        "World Model JEPA Trainer Tick\n\
         =============================\n\
         Decision: {:?}\n\
         Should train: {}\n\
         Trigger: {}\n\
         Rows loaded: {}\n\
         Examples: {}\n\
         Candidate: {}\n\
         Loss total: {}\n\
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
        checkpoint_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "none".into())
    ))
}

