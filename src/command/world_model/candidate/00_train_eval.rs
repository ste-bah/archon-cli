use std::path::Path;

use anyhow::{Result, bail};
use chrono::Utc;

use archon_world_model::embedding::{DeterministicHashEmbeddingAdapter, MemoryEmbeddingAdapter};
use archon_world_model::eval::{
    BrierImprovementReport, PromotionGateReport, evaluate_auxiliary_label_brier,
    evaluate_brier_improvement, evaluate_next_state_cosine_gate, evaluate_surprise_ks_gate,
};
use archon_world_model::jepa::{
    EvalRunStage, JEPA_MODEL_KIND, JepaEvalRecord, JepaPromotionGateReport,
    JepaRepresentationComparisonReport, PersistedEvalMode,
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
        "World Model JEPA-Inspired Train\n\
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
         Loss JEPA-inspired: {:.4}\n\
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

/// Entry point for `archon world eval-jepa` with all v0.4.1 CLI flags.
/// TASK-JEVAL-023; CRIT-4 Tier-0 quick-mode short-circuit wired here.
///
/// Default (no --full): builds JepaEvalGateConfig from config, loads the
/// candidate, runs Tier-0 gates. If any Tier-0 gate fails, writes a
/// `baseline_skipped: true` eval record and returns WITHOUT running fastembed.
/// This is the behaviour advertised by the migration notice (DEC-JEVAL-01).
///
/// --full: skip Tier-0 short-circuit and delegate directly to render_eval_jepa.
///
/// Full wiring of --backend (BackendRuntimeResolver) and --no-cache
/// (CachedEmbeddingAdapter write-bypass) is deferred to TASK-JEVAL-025.
#[allow(clippy::too_many_arguments)]
pub(super) fn render_eval_jepa_with_options(
    config: &archon_core::config::ArchonConfig,
    root: &Path,
    candidate_id: &str,
    full: bool,
    background: bool,
    resume: Option<String>,
    backend: Option<String>,
    no_cache: bool,
) -> Result<String> {
    use archon_world_model::jepa::{
        JepaEvalGateConfig, JepaEvalPlanner, JepaPromotionGateReport, PersistedEvalMode,
    };

    // 1. Emit migration notice when called in default (implicit) mode.
    //    Suppressed when operator explicitly chooses --full, --background, or --resume.
    if !full && resume.is_none() && !background {
        eprintln!("{}", archon_world_model::jepa::MIGRATION_NOTICE);
    }

    // 2. Background path — returns a clear deferral error (CRIT-1).
    if background {
        return handle_background_eval(config, root, candidate_id);
    }

    // 3. Resume path: look up run record, validate preconditions.
    if let Some(ref run_id) = resume {
        return handle_resume_eval(config, root, candidate_id, run_id, no_cache);
    }

    // 4. Emit visible warnings for deferred --backend / --no-cache flags (CRIT-3).
    if let Some(backend_str) = backend.as_deref() {
        eprintln!(
            "⚠ --backend {backend_str} is parsed but not yet wired through the eval pipeline. \
             Eval will use the candidate's default backend. Backend selection via \
             BackendRuntimeResolver is staged for future integration."
        );
        tracing::warn!(
            backend_override = backend_str,
            candidate = candidate_id,
            "CLI --backend flag ignored — eval pipeline not yet wired to BackendRuntimeResolver"
        );
    }
    if no_cache {
        eprintln!(
            "⚠ --no-cache is parsed but not yet wired through the eval pipeline. \
             The embedding cache will still be used per [learning.world_model.jepa.eval] config."
        );
    }

    // 5. CRIT-4: In quick mode (default), run Tier-0 gates before touching fastembed.
    //    Tier-0 operates on candidate metadata only — no rows needed, no embedding call.
    if !full {
        let jepa = &config.learning.world_model.jepa;
        let gate_config = JepaEvalGateConfig {
            min_training_examples: jepa.min_training_examples,
            min_heldout_examples: jepa.min_heldout_examples,
            min_cuda_validation_examples: jepa.min_cuda_validation_examples,
            min_metal_validation_examples: jepa.min_metal_validation_examples,
            backend_parity_cosine_floor: jepa.backend_parity_cosine_floor,
            require_native_accelerator_ops: jepa.require_native_accelerator_ops,
            allow_accelerated_candidate_cpu_stage: jepa.allow_accelerated_candidate_cpu_stage,
            max_checkpoint_mb: jepa.max_checkpoint_mb,
        };
        let registry = archon_world_model::registry::ModelRegistry::open(root)?;
        let candidate = registry.load_jepa_candidate(candidate_id)?;
        let tier0_result = JepaEvalPlanner::evaluate_tier0(&candidate, &gate_config);

        if !tier0_result.all_passed {
            // Tier-0 failed: skip fastembed entirely, write a skipped eval record.
            let config_fingerprint =
                compute_config_fingerprint(&config.learning.world_model.jepa);
            let eval_schema_version = jepa.eval_schema_version_or_default();
            let skipped_reason = format!(
                "Tier-0 gates failed: {:?}",
                tier0_result.failures
            );
            let gates = JepaPromotionGateReport::quick_mode_skipped(
                // corpus_sufficient: use training-time example count vs config threshold
                (candidate.model.metadata.example_count as usize)
                    >= jepa.min_training_examples,
                // representation_collapse: from training outcome
                candidate.outcome.collapse.passes,
                // multi_horizon_consistency: from training outcome
                candidate.outcome.horizon.passes,
                // checkpoint_size: check inline since we already have the candidate
                checkpoint_under_jepa_cap(config, &candidate).unwrap_or(false),
                // tensor_safety
                candidate.model.validate_finite().is_ok(),
                // backend_execution: use Tier-0 backend gate (already checked above)
                archon_world_model::jepa::jepa_backend_promotion_gate_failure(
                    &candidate.model.metadata,
                    jepa.min_cuda_validation_examples,
                    jepa.min_metal_validation_examples,
                ).is_none(),
            );
            let record = JepaEvalRecord {
                candidate_id: candidate_id.to_string(),
                mode: PersistedEvalMode::Quick,
                baseline_skipped: true,
                skipped_reason: Some(skipped_reason.clone()),
                corpus_fingerprint: None, // rows not loaded — correct for a skipped record
                config_fingerprint,
                eval_schema_version,
                comparison: None,
                collapse: candidate.outcome.collapse.clone(),
                horizon: candidate.outcome.horizon.clone(),
                gates,
                created_at: Utc::now(),
            };
            registry.write_jepa_eval_report(&record)?;
            return Ok(format!(
                "Quick eval rejected (Tier-0 failure): {}\n\
                 Skipped fastembed baseline — no expensive embedding call made.\n\
                 Eval record written with baseline_skipped=true.\n\
                 Re-run with --full to force full evaluation regardless of Tier-0 results.",
                skipped_reason
            ));
        }
    }

    // 6. Full mode OR Tier-0 passed in quick mode: delegate to the full pipeline.
    render_eval_jepa(config, root, candidate_id)
}

/// Handles `--background`: returns a clear deferral error.
///
/// CRIT-1 fix: the previous implementation called `spawn_background_worker`
/// which re-execs with `--__bg-worker --run-id <id>` flags that are NOT wired
/// into the CLI dispatch — the grandchild would die immediately on arg parsing.
///
/// This honest deferral prevents silent breakage while the background worker
/// entry point is staged for a future task. Operators are given a clear
/// workaround (foreground mode with shell backgrounding or tmux).
fn handle_background_eval(
    _config: &archon_core::config::ArchonConfig,
    _root: &Path,
    _candidate_id: &str,
) -> Result<String> {
    anyhow::bail!(
        "Background eval execution requires a background-worker entry point that \
         has not yet been wired into the CLI. Use foreground mode (omit --background) \
         for now. The --background flag will be fully wired in a future task.\n\n\
         Workaround: run `archon world eval-jepa <candidate>` in the foreground; \
         to release the terminal, use shell backgrounding or `nohup`/`tmux`."
    )
}

/// Handles `--resume <run-id>`: validates run record and cache preconditions.
/// Full resume logic (driving the pipeline from `current_stage`) is deferred.
fn handle_resume_eval(
    _config: &archon_core::config::ArchonConfig,
    root: &Path,
    candidate_id: &str,
    run_id: &str,
    no_cache: bool,
) -> Result<String> {
    let run_dir = root.join("jepa/eval-runs");
    let store = JepaEvalRunStore::new(run_dir)?;
    let record = store.read_run(run_id).map_err(|e| {
        anyhow::anyhow!(
            "Cannot resume run {run_id}: {e}. \
             List all runs: archon world eval-jepa-runs"
        )
    })?;

    // Validate that the run belongs to the requested candidate.
    if record.candidate_id != candidate_id {
        anyhow::bail!(
            "Run {run_id} belongs to candidate '{}', not '{candidate_id}'",
            record.candidate_id
        );
    }

    // F-HIGH-02: resuming a BaselineEmbed-stage run requires the embedding cache.
    if matches!(record.current_stage, EvalRunStage::BaselineEmbed) && no_cache {
        anyhow::bail!(
            "--resume of a baseline_embed-stage run requires the embedding cache. \
             Remove --no-cache flag. Run ID: {run_id} (ERR-JEVAL-07)"
        );
    }

    // Full resume wiring is deferred; return current status as confirmation.
    Ok(format!(
        "Resume of run {run_id} (candidate '{candidate_id}') — full resume logic deferred.\n\
         Current stage: {:?}, status: {:?}\n\
         Re-run without --resume to start fresh, or track progress with:\n\
         archon world eval-jepa-status {run_id}\n",
        record.current_stage, record.status
    ))
}

pub(super) fn render_eval_jepa(
    config: &archon_core::config::ArchonConfig,
    root: &Path,
    candidate_id: &str,
) -> Result<String> {
    // CRIT-6: stage-boundary progress prints so operator can observe long evals.
    // ForegroundProgressReporter (T013) is scaffolding for future per-row ticks;
    // these eprintln! calls cover the coarse stage boundaries without it.
    let stage_start = std::time::Instant::now();

    eprintln!("Stage: load_candidate");
    let registry = ModelRegistry::open(root)?;
    let candidate = registry.load_jepa_candidate(candidate_id)?;

    eprintln!(
        "Stage: load_rows ({:.1}s elapsed)",
        stage_start.elapsed().as_secs_f64()
    );
    let rows = WorldModelStore::open(root)?.load_rows()?;

    // Compute proper fingerprints before constructing the eval record.
    // corpus_fingerprint: hash of row content — catches corpus mutations since eval.
    let corpus_fingerprint =
        Some(archon_world_model::jepa::JepaEvalPlanner::compute_corpus_fingerprint(&rows));
    // config_fingerprint: uses the same function as the promotion guard (02_promote_helpers.rs)
    // so that check 6 in check_pre_promotion_conditions always matches.
    let config_fingerprint = compute_config_fingerprint(&config.learning.world_model.jepa);
    // eval_schema_version: stored standalone for cheap pre-check at promotion time (PRD §11).
    let eval_schema_version = config
        .learning
        .world_model
        .jepa
        .eval_schema_version_or_default();

    eprintln!(
        "Stage: baseline_embed (corpus rows: {}, {:.1}s elapsed)",
        rows.len(),
        stage_start.elapsed().as_secs_f64()
    );
    let comparison =
        compare_jepa_representations(config, &candidate.model, &rows, "fastembed", true);
    let comparison_path = registry.write_jepa_representation_comparison(&comparison)?;

    eprintln!(
        "Stage: gates ({:.1}s elapsed)",
        stage_start.elapsed().as_secs_f64()
    );
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
        mode: PersistedEvalMode::Full,
        baseline_skipped: false,
        skipped_reason: None,
        corpus_fingerprint,   // populated from actual corpus hash
        config_fingerprint,   // populated from current config (matches promotion guard)
        eval_schema_version,  // populated from config (not hardcoded 0)
        comparison: Some(comparison),
        collapse: candidate.outcome.collapse.clone(),
        horizon: candidate.outcome.horizon.clone(),
        gates,
        created_at: Utc::now(),
    };

    eprintln!(
        "Stage: report (total: {:.1}s)",
        stage_start.elapsed().as_secs_f64()
    );
    let eval_path = registry.write_jepa_eval_report(&record)?;

    Ok(format!(
        "World Model JEPA-Inspired Eval\n\
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
        record.comparison.as_ref().map(|c| c.heldout_examples).unwrap_or(0),
        record.comparison.as_ref().map(|c| c.baseline_backend.as_str()).unwrap_or("none"),
        record.comparison.as_ref().map(|c| c.baseline_available).unwrap_or(false),
        record.comparison.as_ref().map(|c| c.relative_improvement * 100.0).unwrap_or(0.0),
        record.comparison.as_ref().map(|c| c.brier_regressed).unwrap_or(false),
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
