fn split_for_eval(
    examples: &[LatentTransitionExample],
) -> Result<(&[LatentTransitionExample], &[LatentTransitionExample])> {
    if examples.len() < 2 {
        bail!("eval requires at least two transition examples");
    }
    let split = ((examples.len() as f32) * 0.8).floor() as usize;
    let split = split.clamp(1, examples.len() - 1);
    Ok((&examples[..split], &examples[split..]))
}

fn selected_training_backend(
    config: &archon_core::config::ArchonConfig,
) -> archon_world_model::backend::BackendStatus {
    archon_world_model::backend::select_runtime_backend(
        match config.learning.world_model.training.backend.as_str() {
            "cpu" => archon_world_model::backend::BackendKind::Cpu,
            "cuda" => archon_world_model::backend::BackendKind::Cuda,
            "metal" => archon_world_model::backend::BackendKind::Metal,
            _ => archon_world_model::backend::BackendKind::Auto,
        },
        config.learning.world_model.training.allow_cpu_fallback,
    )
}

fn enforce_accelerator_cap(
    config: &archon_core::config::ArchonConfig,
    model: &CpuLatentTransitionModel,
    backend: &archon_world_model::backend::BackendStatus,
) -> Result<()> {
    if backend.selected == archon_world_model::backend::BackendKind::Cpu {
        return Ok(());
    }
    let estimated_mb = model
        .metadata
        .parameter_count
        .saturating_mul(4)
        .div_ceil(1024 * 1024);
    if estimated_mb
        > config
            .learning
            .world_model
            .training
            .max_accelerator_memory_mb
    {
        bail!(
            "world-model checkpoint estimate {estimated_mb}MB exceeds max_accelerator_memory_mb={}",
            config
                .learning
                .world_model
                .training
                .max_accelerator_memory_mb
        );
    }
    Ok(())
}

fn enforce_jepa_checkpoint_cap(
    config: &archon_core::config::ArchonConfig,
    model: &archon_world_model::jepa::JepaTraceModel,
) -> Result<()> {
    let estimated_mb = model
        .metadata
        .parameter_count
        .saturating_mul(4)
        .div_ceil(1024 * 1024);
    if estimated_mb > config.learning.world_model.jepa.max_checkpoint_mb {
        bail!(
            "jepa checkpoint estimate {estimated_mb}MB exceeds max_checkpoint_mb={}",
            config.learning.world_model.jepa.max_checkpoint_mb
        );
    }
    Ok(())
}

fn jepa_training_config(
    config: &archon_core::config::ArchonConfig,
) -> Result<archon_world_model::jepa::JepaTrainingConfig> {
    let jepa = &config.learning.world_model.jepa;
    let training = archon_world_model::jepa::JepaTrainingConfig {
        latent_dim: jepa.latent_dim,
        context_window_rows: jepa.context_window_rows,
        target_window_rows: jepa.target_window_rows,
        prediction_horizons: jepa.prediction_horizons.clone(),
        mask_ratio: jepa.mask_ratio,
        ema_decay: jepa.ema_decay,
        latent_var_floor: jepa.latent_var_floor,
        max_epochs: jepa.max_epochs,
        learning_rate: jepa.learning_rate,
        alpha_mse: jepa.alpha_mse,
        beta_aux: jepa.beta_aux,
        gamma_horizon: jepa.gamma_horizon,
        delta_var: jepa.delta_var,
        min_latent_std: jepa.min_latent_std,
        min_effective_rank_ratio: jepa.min_effective_rank_ratio,
        horizon_consistency_tol: jepa.horizon_consistency_tol,
    };
    training.validate()?;
    Ok(training)
}

fn compare_jepa_representations(
    config: &archon_core::config::ArchonConfig,
    candidate: &archon_world_model::jepa::JepaTraceModel,
    rows: &[archon_world_model::WorldTraceRow],
    baseline: &str,
    promotion_gating: bool,
) -> JepaRepresentationComparisonReport {
    match compare_jepa_representations_inner(config, candidate, rows, baseline, promotion_gating) {
        Ok(report) => report,
        Err(error) => JepaRepresentationComparisonReport::fail_closed(
            candidate.metadata.model_id.clone(),
            if promotion_gating {
                "fastembed"
            } else {
                baseline
            }
            .to_string(),
            error.to_string(),
            config.learning.world_model.jepa.min_heldout_examples,
            config.learning.world_model.jepa.min_baseline_improvement,
        ),
    }
}

fn compare_jepa_representations_inner(
    config: &archon_core::config::ArchonConfig,
    candidate: &archon_world_model::jepa::JepaTraceModel,
    rows: &[archon_world_model::WorldTraceRow],
    baseline: &str,
    promotion_gating: bool,
) -> Result<JepaRepresentationComparisonReport> {
    let baseline_backend = if promotion_gating {
        "fastembed"
    } else {
        baseline
    };
    let state_dim = config.learning.world_model.state_dim;
    let jepa_examples =
        archon_world_model::train::examples_from_rows_with_representation_adapter(rows, candidate)?;
    let baseline_adapter = baseline_representation_adapter(baseline_backend, state_dim)?;
    let baseline_examples =
        archon_world_model::train::examples_from_rows_with_representation_adapter(
            rows,
            baseline_adapter.as_ref(),
        )?;
    if jepa_examples.len() != baseline_examples.len() {
        bail!("jepa and baseline example counts differ");
    }
    let (jepa_train, jepa_heldout) = split_for_eval(&jepa_examples)?;
    let (baseline_train, baseline_heldout) = split_for_eval(&baseline_examples)?;
    if promotion_gating
        && jepa_heldout.len() < config.learning.world_model.jepa.min_heldout_examples
    {
        bail!(
            "heldout example count {} is below min_heldout_examples={}",
            jepa_heldout.len(),
            config.learning.world_model.jepa.min_heldout_examples
        );
    }

    let jepa_transition = CpuLatentTransitionModel::fit(state_dim, jepa_train)?;
    let baseline_transition = CpuLatentTransitionModel::fit(state_dim, baseline_train)?;
    let jepa_error = jepa_transition.mean_cosine_error(jepa_heldout)?;
    let baseline_error = baseline_transition.mean_cosine_error(baseline_heldout)?;
    let jepa_similarity = 1.0 - jepa_error;
    let baseline_similarity = 1.0 - baseline_error;
    let relative_improvement = if baseline_similarity.abs() <= f32::EPSILON {
        0.0
    } else {
        (jepa_similarity - baseline_similarity) / baseline_similarity.abs()
    };
    let jepa_brier = mean_auxiliary_brier(&jepa_transition, jepa_heldout)?;
    let baseline_brier = mean_auxiliary_brier(&baseline_transition, baseline_heldout)?;
    let brier_regressed = jepa_brier > baseline_brier + 0.0001;
    let passed = jepa_heldout.len() >= config.learning.world_model.jepa.min_heldout_examples
        && relative_improvement >= config.learning.world_model.jepa.min_baseline_improvement
        && !brier_regressed;

    Ok(JepaRepresentationComparisonReport {
        candidate_id: candidate.metadata.model_id.clone(),
        baseline_backend: baseline_backend.to_string(),
        baseline_available: true,
        failure_reason: None,
        heldout_examples: jepa_heldout.len(),
        min_heldout_examples: config.learning.world_model.jepa.min_heldout_examples,
        jepa_next_state_cosine_similarity: jepa_similarity,
        baseline_next_state_cosine_similarity: baseline_similarity,
        relative_improvement,
        min_baseline_improvement: config.learning.world_model.jepa.min_baseline_improvement,
        brier_regressed,
        passed,
    })
}

fn baseline_representation_adapter(
    baseline: &str,
    state_dim: usize,
) -> Result<Box<dyn archon_world_model::WorldRepresentationAdapter>> {
    // Load policy fail-closed; allow_embedding_cache defaults to false.
    let policy =
        archon_policy::load_effective_policy(&std::env::current_dir()?).unwrap_or_default();
    let allow_cache = policy.world_model.allow_embedding_cache;
    match baseline {
        "fastembed" => {
            let inner = Box::new(MemoryEmbeddingAdapter::local_fastembed(state_dim)?);
            let cache_config = archon_world_model::embedding::EmbeddingCacheConfig {
                cache_dir: super::world_model_root()?
                    .join("embeddings")
                    .join("cache"),
                cache_enabled: allow_cache,
                cache_max_bytes: 256 * 1024 * 1024, // 256 MiB default
                redact_before_embedding: true,
                // TODO(T025): read from config.learning.world_model.jepa.eval.eval_schema_version
                eval_schema_version: 1u32,
            };
            let cached = Box::new(archon_world_model::embedding::CachedEmbeddingAdapter::new(
                inner,
                cache_config,
                allow_cache,
            ));
            Ok(Box::new(GenericEmbeddingRepresentationAdapter::new(cached)))
        }
        "deterministic-hash" => Ok(Box::new(GenericEmbeddingRepresentationAdapter::new(
            Box::new(DeterministicHashEmbeddingAdapter::new(state_dim)?),
        ))),
        _ => bail!("unsupported representation baseline: {baseline}"),
    }
}
