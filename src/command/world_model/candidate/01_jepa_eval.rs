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
        "World Model JEPA-Inspired Inspect\n\
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
         JEPA-inspired cosine similarity: {:.4}\n\
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
