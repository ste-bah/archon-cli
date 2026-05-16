use std::path::Path;

use anyhow::{Result, bail};
use chrono::Utc;

use archon_world_model::embedding::{DeterministicHashEmbeddingAdapter, MemoryEmbeddingAdapter};
use archon_world_model::eval::{
    BrierImprovementReport, PromotionGateReport, evaluate_auxiliary_label_brier,
    evaluate_brier_improvement, evaluate_next_state_cosine_gate, evaluate_surprise_ks_gate,
};
use archon_world_model::jepa::{
    JEPA_MODEL_KIND, JepaEvalRecord, JepaPromotionGateReport, JepaRepresentationComparisonReport,
};
use archon_world_model::model::{CpuLatentTransitionModel, LatentTransitionExample};
use archon_world_model::registry::{CandidateEvalRecord, JepaCandidateRecord, ModelRegistry};
use archon_world_model::representation::GenericEmbeddingRepresentationAdapter;
use archon_world_model::schema::WorldLabelSet;
use archon_world_model::storage::WorldModelStore;

use super::embedding_runtime::build_embedding_adapter;

pub(super) fn render_train(
    config: &archon_core::config::ArchonConfig,
    root: &Path,
    candidate_flag: bool,
    max_runtime_ms: Option<u64>,
) -> Result<String> {
    let backend = selected_training_backend(config);
    let rows = WorldModelStore::open(root)?.load_rows()?;
    let state_dim = config.learning.world_model.state_dim;
    let adapter = build_embedding_adapter(config)?;
    let examples =
        archon_world_model::train::examples_from_rows_with_adapter(&rows, adapter.as_ref())?;
    if examples.is_empty() {
        bail!("not enough rows to train: need at least two rows in the same session");
    }

    let (model, outcome) = archon_world_model::train::train_candidate_with_backend_or_cpu_fallback(
        state_dim,
        &examples,
        backend.selected,
        config.learning.world_model.training.allow_cpu_fallback,
    )?;
    enforce_accelerator_cap(config, &model, &backend)?;
    let registry = ModelRegistry::open(root)?;
    let path = registry.write_candidate(&model, &outcome)?;
    let max_runtime_ms =
        max_runtime_ms.unwrap_or(config.learning.world_model.training.max_runtime_ms);

    Ok(format!(
        "World Model Train\n\
         =================\n\
         Candidate mode: forced{}\n\
         Candidate: {}\n\
         Backend: {}\n\
         Backend fallback: {}\n\
         Rows loaded: {}\n\
         Examples: {}\n\
         Parameters: {}\n\
         Mean cosine error: {:.4}\n\
         Max runtime ms: {}\n\
         Checkpoint: {}",
        if candidate_flag { " (--candidate)" } else { "" },
        model.metadata.model_id,
        backend.selected,
        backend.fallback_reason.as_deref().unwrap_or("none"),
        rows.len(),
        examples.len(),
        model.metadata.parameter_count,
        outcome.training_mean_cosine_error,
        max_runtime_ms,
        path.display()
    ))
}

pub(super) fn render_train_jepa(
    config: &archon_core::config::ArchonConfig,
    root: &Path,
    candidate_flag: bool,
    max_runtime_ms: Option<u64>,
) -> Result<String> {
    let rows = WorldModelStore::open(root)?.load_rows()?;
    let jepa_config = jepa_training_config(config)?;
    let state_dim = config.learning.world_model.state_dim;
    if jepa_config.latent_dim != state_dim {
        bail!(
            "jepa latent_dim ({}) must equal active transition state_dim ({state_dim})",
            jepa_config.latent_dim
        );
    }

    let backend = selected_training_backend(config);
    let (model, outcome) = archon_world_model::jepa::train_jepa_candidate_with_backend(
        &rows,
        &jepa_config,
        backend.requested,
        config.learning.world_model.training.allow_cpu_fallback,
    )?;
    enforce_jepa_checkpoint_cap(config, &model)?;
    let registry = ModelRegistry::open(root)?;
    let manifest_path = registry.write_jepa_candidate(&model, &outcome)?;
    let checkpoint_path = registry
        .load_jepa_candidate(&model.metadata.model_id)?
        .checkpoint
        .path;
    let max_runtime_ms = max_runtime_ms.unwrap_or(config.learning.world_model.jepa.max_runtime_ms);

    Ok(format!(
        "World Model JEPA Train\n\
         =======================\n\
         Candidate mode: forced{}\n\
         Candidate: {}\n\
         Model kind: {}\n\
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
         Latent dim: {}\n\
         Rows loaded: {}\n\
         Examples: {}\n\
         Horizons: {:?}\n\
         Parameters: {}\n\
         Loss total: {:.4}\n\
         Loss JEPA: {:.4}\n\
         Loss MSE: {:.4}\n\
         Loss aux: {:.4}\n\
         Loss horizon: {:.4}\n\
         Loss variance: {:.4}\n\
         Loss improved: {}\n\
         Masked fields: context={} action={}\n\
         Collapse gate: {} (std={:.4}, rank_ratio={:.4})\n\
         Horizon gate: {}\n\
         Max runtime ms: {}\n\
         Manifest: {}\n\
         Checkpoint: {}",
        if candidate_flag { " (--candidate)" } else { "" },
        model.metadata.model_id,
        model.metadata.model_kind,
        outcome.metadata.backend_execution.requested_backend,
        outcome.metadata.backend_execution.selected_backend,
        outcome.metadata.backend_execution.framework,
        outcome
            .metadata
            .backend_execution
            .device_name
            .as_deref()
            .unwrap_or("unknown"),
        outcome
            .metadata
            .backend_execution
            .fallback_reason
            .as_deref()
            .unwrap_or("none"),
        outcome.metadata.backend_execution.native_encode,
        outcome.metadata.backend_execution.native_predictor_fit,
        outcome.metadata.backend_execution.native_auxiliary_fit,
        outcome.metadata.backend_execution.native_transition_fit,
        outcome.metadata.backend_execution.native_loss_eval,
        outcome
            .metadata
            .backend_execution
            .native_runtime_prediction
            .unwrap_or(false),
        outcome.metadata.backend_execution.host_fallback_count,
        outcome
            .metadata
            .backend_execution
            .hardware_validation_captured_at
            .is_some(),
        outcome.metadata.backend_execution.validation_example_count,
        model.metadata.latent_dim,
        rows.len(),
        model.metadata.example_count,
        model.metadata.prediction_horizons,
        model.metadata.parameter_count,
        outcome.losses.loss_total,
        outcome.losses.loss_jepa,
        outcome.losses.loss_mse,
        outcome.losses.loss_aux,
        outcome.losses.loss_horizon,
        outcome.losses.loss_var,
        outcome.progress.improved,
        outcome.masking.masked_context_fields,
        outcome.masking.masked_action_fields,
        outcome.collapse.passes,
        outcome.collapse.mean_latent_std,
        outcome.collapse.effective_rank_ratio,
        outcome.horizon.passes,
        max_runtime_ms,
        manifest_path.display(),
        checkpoint_path.display()
    ))
}

pub(super) fn render_eval(
    config: &archon_core::config::ArchonConfig,
    root: &Path,
    candidate_id: Option<&str>,
) -> Result<String> {
    let candidate_id = candidate_id.ok_or_else(|| anyhow::anyhow!("candidate id is required"))?;
    let registry = ModelRegistry::open(root)?;
    let candidate = registry.load_cpu_candidate(candidate_id)?;
    let rows = WorldModelStore::open(root)?.load_rows()?;
    let adapter = build_embedding_adapter(config)?;
    let examples =
        archon_world_model::train::examples_from_rows_with_adapter(&rows, adapter.as_ref())?;
    let (baseline, heldout) = split_for_eval(&examples)?;
    let eval = &config.learning.world_model.eval;

    let next_state = evaluate_next_state_cosine_gate(
        &candidate.model,
        baseline,
        heldout,
        eval.next_state_baseline_min_delta,
    )?;
    let candidate_distances = model_cosine_distances(&candidate.model, heldout)?;
    let actual_distances = actual_transition_distances(heldout)?;
    let surprise = evaluate_surprise_ks_gate(
        &candidate_distances,
        &actual_distances,
        eval.surprise_ks_min_p,
    )?;
    let brier_scores = evaluate_auxiliary_label_brier(&candidate.model, baseline, heldout)?;
    let brier = evaluate_brier_improvement(&brier_scores, 0.02, 3);
    let no_critical_regression = candidate
        .model
        .mean_delta
        .iter()
        .all(|value| value.is_finite())
        && candidate
            .model
            .state_weights
            .iter()
            .all(|value| value.is_finite())
        && candidate
            .model
            .action_weights
            .iter()
            .all(|value| value.is_finite())
        && candidate_distances.iter().all(|value| value.is_finite());
    let report = PromotionGateReport::from_component_gates(
        &next_state,
        &surprise,
        counterfactual_gate_from_heldout(heldout, eval.counterfactual_ndcg_min),
        &brier,
        no_critical_regression,
    );
    let record = CandidateEvalRecord {
        candidate_id: candidate_id.to_string(),
        report,
        next_state: Some(next_state),
        surprise: Some(surprise),
        brier: Some(brier),
        created_at: Utc::now(),
    };
    let path = registry.write_eval_report(&record)?;

    Ok(format!(
        "World Model Eval\n\
         =================\n\
         Candidate: {candidate_id}\n\
         Examples: {}\n\
         Heldout: {}\n\
         Next-state improvement: {:.2}%\n\
         Surprise KS p: {:.4}\n\
         Brier labels improved: {}/{}\n\
         Primary gates pass: {}\n\
         Gate policy: all primary gates are mandatory\n\
         Eval report: {}",
        examples.len(),
        heldout.len(),
        record
            .next_state
            .as_ref()
            .map(|next| next.relative_improvement * 100.0)
            .unwrap_or_default(),
        record
            .surprise
            .as_ref()
            .map(|surprise| surprise.p_value)
            .unwrap_or_default(),
        record
            .brier
            .as_ref()
            .map(|brier| brier.labels_improved)
            .unwrap_or_default(),
        record
            .brier
            .as_ref()
            .map(|brier| brier.total_labels)
            .unwrap_or_default(),
        record.report.all_primary_gates_passed(),
        path.display()
    ))
}

pub(super) fn render_eval_jepa(
    config: &archon_core::config::ArchonConfig,
    root: &Path,
    candidate_id: &str,
) -> Result<String> {
    let registry = ModelRegistry::open(root)?;
    let candidate = registry.load_jepa_candidate(candidate_id)?;
    let rows = WorldModelStore::open(root)?.load_rows()?;
    let comparison =
        compare_jepa_representations(config, &candidate.model, &rows, "fastembed", true);
    let comparison_path = registry.write_jepa_representation_comparison(&comparison)?;
    let checkpoint_size_passed = checkpoint_under_jepa_cap(config, &candidate)?;
    let tensor_safety = candidate.model.validate_finite().is_ok();
    let corpus_sufficient = candidate.model.metadata.example_count as usize
        >= config.learning.world_model.jepa.min_training_examples;
    let backend_gate_failure = archon_world_model::jepa::jepa_backend_promotion_gate_failure(
        &candidate.model.metadata,
        config
            .learning
            .world_model
            .jepa
            .min_cuda_validation_examples,
        config
            .learning
            .world_model
            .jepa
            .min_metal_validation_examples,
    );
    let parity_passed = backend_gate_failure.is_none()
        && jepa_backend_forward_parity_passes(config, &candidate.model, &rows);
    let backend_execution = backend_gate_failure.is_none() && parity_passed;
    let backend_gate_reason = backend_gate_failure
        .or_else(|| (!parity_passed).then_some("JepaBackendParityFailed"))
        .unwrap_or("none");
    let gates = JepaPromotionGateReport::from_parts_with_backend_execution(
        corpus_sufficient,
        comparison.passed,
        candidate.outcome.collapse.passes,
        candidate.outcome.horizon.passes,
        checkpoint_size_passed,
        tensor_safety,
        backend_execution,
    );
    let record = JepaEvalRecord {
        candidate_id: candidate_id.to_string(),
        comparison,
        collapse: candidate.outcome.collapse.clone(),
        horizon: candidate.outcome.horizon.clone(),
        gates,
        created_at: Utc::now(),
    };
    let eval_path = registry.write_jepa_eval_report(&record)?;

    Ok(format!(
        "World Model JEPA Eval\n\
         =====================\n\
         Candidate: {candidate_id}\n\
         Model kind: {}\n\
         Examples: {}\n\
         Heldout: {}\n\
         Baseline: {}\n\
         Baseline available: {}\n\
         Relative improvement: {:.2}%\n\
         Brier regressed: {}\n\
         Corpus sufficient: {}\n\
         Collapse gate: {}\n\
         Horizon gate: {}\n\
         Checkpoint size gate: {}\n\
         Tensor safety gate: {}\n\
         Backend execution gate: {}\n\
         Backend gate reason: {}\n\
         Primary gates pass: {}\n\
         Eval report: {}\n\
         Comparison report: {}",
        candidate.model.metadata.model_kind,
        candidate.model.metadata.example_count,
        record.comparison.heldout_examples,
        record.comparison.baseline_backend,
        record.comparison.baseline_available,
        record.comparison.relative_improvement * 100.0,
        record.comparison.brier_regressed,
        record.gates.corpus_sufficient,
        record.gates.representation_collapse,
        record.gates.multi_horizon_consistency,
        record.gates.checkpoint_size,
        record.gates.tensor_safety,
        record.gates.backend_execution,
        backend_gate_reason,
        record.gates.passed,
        eval_path.display(),
        comparison_path.display()
    ))
}

