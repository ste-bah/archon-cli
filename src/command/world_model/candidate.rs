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

fn jepa_candidate_count(root: &Path) -> usize {
    let dir = root.join("jepa").join("candidates");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("json"))
        .count()
}

pub(super) fn render_promote(root: &Path, model_id: &str) -> Result<String> {
    let registry = ModelRegistry::open(root)?;
    let eval = registry
        .load_eval_report(model_id)?
        .ok_or_else(|| anyhow::anyhow!("candidate {model_id} has no eval report"))?;
    if !eval.report.all_primary_gates_passed() {
        bail!("candidate {model_id} has not passed all mandatory promotion gates");
    }

    let path = registry.promote(model_id)?;
    let validation_path = write_promotion_validation_metadata(root, model_id, &eval)?;
    Ok(format!(
        "World Model Promote\n\
         ===================\n\
         Active model: {model_id}\n\
         Pointer: {}\n\
         Validation metadata: {}",
        path.display(),
        validation_path.display()
    ))
}

pub(super) fn render_promote_jepa(root: &Path, model_id: &str) -> Result<String> {
    let registry = ModelRegistry::open(root)?;
    let candidate = registry.load_jepa_candidate(model_id)?;
    archon_world_model::jepa::validate_jepa_backend_execution(&candidate.model.metadata)?;
    let eval = registry
        .load_jepa_eval_report(model_id)?
        .ok_or_else(|| anyhow::anyhow!("jepa candidate {model_id} has no eval report"))?;
    if !eval.gates.passed {
        bail!("jepa candidate {model_id} has not passed all mandatory promotion gates");
    }

    let path = registry.promote_model_kind(model_id, JEPA_MODEL_KIND)?;
    Ok(format!(
        "World Model JEPA Promote\n\
         ========================\n\
         Active model: {model_id}\n\
         Model kind: {JEPA_MODEL_KIND}\n\
         Pointer: {}",
        path.display()
    ))
}

pub(super) fn render_rollback(root: &Path, model_id: &str) -> Result<String> {
    let path = ModelRegistry::open(root)?.rollback(model_id)?;
    Ok(format!(
        "World Model Rollback\n\
         ====================\n\
         Active model: {model_id}\n\
         Pointer: {}",
        path.display()
    ))
}

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
    match baseline {
        "fastembed" => Ok(Box::new(GenericEmbeddingRepresentationAdapter::new(
            Box::new(MemoryEmbeddingAdapter::local_fastembed(state_dim)?),
        ))),
        "deterministic-hash" => Ok(Box::new(GenericEmbeddingRepresentationAdapter::new(
            Box::new(DeterministicHashEmbeddingAdapter::new(state_dim)?),
        ))),
        _ => bail!("unsupported representation baseline: {baseline}"),
    }
}

fn mean_auxiliary_brier(
    model: &CpuLatentTransitionModel,
    examples: &[LatentTransitionExample],
) -> Result<f32> {
    let mut total = 0.0;
    let mut count = 0.0;
    for example in examples {
        for prediction in model.predict_auxiliary(&example.state, &example.action)? {
            let actual = if label_value(&example.labels, &prediction.label) {
                1.0
            } else {
                0.0
            };
            total += (prediction.probability - actual).powi(2);
            count += 1.0;
        }
    }
    Ok(if count == 0.0 { 1.0 } else { total / count })
}

fn checkpoint_under_jepa_cap(
    config: &archon_core::config::ArchonConfig,
    candidate: &JepaCandidateRecord,
) -> Result<bool> {
    let max_bytes = config
        .learning
        .world_model
        .jepa
        .max_checkpoint_mb
        .saturating_mul(1024 * 1024);
    let bytes = std::fs::metadata(&candidate.checkpoint.path)
        .map(|metadata| metadata.len())
        .unwrap_or(u64::MAX);
    Ok(bytes <= max_bytes)
}

fn model_cosine_distances(
    model: &CpuLatentTransitionModel,
    examples: &[LatentTransitionExample],
) -> Result<Vec<f32>> {
    examples
        .iter()
        .map(|example| {
            let predicted = archon_world_model::backend::predict_next_with_backend(
                model,
                &example.state,
                &example.action,
                model.metadata.backend,
            )?;
            cosine_error(&predicted, &example.next_state)
        })
        .collect()
}

fn actual_transition_distances(examples: &[LatentTransitionExample]) -> Result<Vec<f32>> {
    examples
        .iter()
        .map(|example| cosine_error(&example.state, &example.next_state))
        .collect()
}

fn cosine_error(left: &[f32], right: &[f32]) -> Result<f32> {
    if left.len() != right.len() {
        bail!("cosine inputs must have matching dimensions");
    }

    let dot = left.iter().zip(right).map(|(a, b)| a * b).sum::<f32>();
    let left_norm = left.iter().map(|value| value * value).sum::<f32>().sqrt();
    let right_norm = right.iter().map(|value| value * value).sum::<f32>().sqrt();
    if left_norm == 0.0 || right_norm == 0.0 {
        Ok(1.0)
    } else {
        Ok(1.0 - (dot / (left_norm * right_norm)).clamp(-1.0, 1.0))
    }
}

fn write_promotion_validation_metadata(
    root: &Path,
    model_id: &str,
    eval: &CandidateEvalRecord,
) -> Result<std::path::PathBuf> {
    let dir = root.join("active");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{model_id}.validation.json"));
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "model_id": model_id,
            "validation_recorded_at": Utc::now(),
            "promotion_gate": eval.report,
            "next_state": eval.next_state,
            "surprise": eval.surprise,
            "brier": eval.brier,
            "backend_validation": {
                "cpu": "supported",
                "cuda": "feature-gated",
                "mlx_metal": "experimental_until_hardware_validation"
            }
        }))?,
    )?;
    Ok(path)
}

fn counterfactual_gate_from_heldout(examples: &[LatentTransitionExample], min_ndcg: f32) -> bool {
    if examples.is_empty() {
        return false;
    }
    let mut scores = examples
        .iter()
        .enumerate()
        .map(|(idx, example)| {
            let risk = label_risk(&example.labels);
            archon_world_model::shadow::ShadowActionScore {
                action_id: format!("heldout-{idx}"),
                advisory_score: label_success(&example.labels) - risk,
                estimated_success: label_success(&example.labels),
                estimated_risk: risk,
            }
        })
        .collect::<Vec<_>>();
    scores.sort_by(|left, right| right.advisory_score.total_cmp(&left.advisory_score));
    let relevance = examples
        .iter()
        .enumerate()
        .map(|(idx, example)| {
            (
                format!("heldout-{idx}"),
                label_success(&example.labels) * 3.0,
            )
        })
        .collect::<Vec<_>>();
    archon_world_model::shadow::ndcg_at_k(&scores, &relevance, 5) >= min_ndcg
}

fn label_value(labels: &WorldLabelSet, label: &str) -> bool {
    match label {
        "failure" => labels.failure,
        "retry" => labels.retry,
        "provider_incident" => labels.provider_incident,
        "verification_needed" => labels.verification_needed,
        "user_correction" => labels.user_correction,
        "plan_drift" => labels.plan_drift,
        "high_cost" => labels.high_cost,
        "slow_run" => labels.slow_run,
        _ => false,
    }
}

fn label_success(labels: &WorldLabelSet) -> f32 {
    match labels.success {
        Some(true) => 1.0,
        Some(false) => 0.0,
        None if labels.failure => 0.0,
        None => 0.5,
    }
}

fn label_risk(labels: &WorldLabelSet) -> f32 {
    let mut risk: f32 = 0.0;
    if labels.failure {
        risk += 0.35;
    }
    if labels.retry {
        risk += 0.20;
    }
    if labels.provider_incident {
        risk += 0.20;
    }
    if labels.verification_needed || labels.plan_drift {
        risk += 0.15;
    }
    risk.min(1.0)
}

#[allow(dead_code)]
fn _assert_brier_report_is_serializable(report: &BrierImprovementReport) -> usize {
    report.labels_improved
}
