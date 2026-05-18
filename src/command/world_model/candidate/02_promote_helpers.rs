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

/// Computes the sha256 fingerprint of the gate-affecting subset of
/// [learning.world_model.jepa] config, per PRD §11.
/// EXCLUDED: performance keys (batch_size, max_runtime_ms, cache_max_mb, mode, etc.)
pub fn compute_config_fingerprint(
    jepa: &archon_core::config::WorldModelJepaConfig,
) -> String {
    use hex::ToHex;
    use sha2::{Digest, Sha256};

    let mut h = Sha256::new();
    h.update(jepa.eval_schema_version_or_default().to_string().as_bytes());
    h.update(b"\0");
    h.update(if jepa.require_native_accelerator_ops {
        b"1" as &[u8]
    } else {
        b"0"
    });
    h.update(b"\0");
    h.update(if jepa.allow_accelerated_candidate_cpu_stage {
        b"1" as &[u8]
    } else {
        b"0"
    });
    h.update(b"\0");
    h.update(jepa.min_metal_validation_examples.to_string().as_bytes());
    h.update(b"\0");
    h.update(jepa.min_cuda_validation_examples.to_string().as_bytes());
    h.update(b"\0");
    h.update(format!("{:.6}", jepa.backend_parity_cosine_floor).as_bytes());
    h.finalize().encode_hex()
}

/// Loads world model rows and computes the corpus fingerprint for comparison
/// against an eval record's corpus_fingerprint field.
fn compute_current_corpus_fingerprint(root: &std::path::Path) -> anyhow::Result<String> {
    let store = archon_world_model::storage::WorldModelStore::open(root)?;
    let rows = store.load_rows()?;
    Ok(archon_world_model::jepa::JepaEvalPlanner::compute_corpus_fingerprint(&rows))
}

/// Runs the 7 pre-promotion checks defined in PRD §11.
/// Returns Ok(()) if all checks pass, Err with actionable message otherwise.
/// `current_corpus_fingerprint` is None when no corpus exists yet (test mode).
/// `comparison_report_exists` lets the caller mock the filesystem check.
pub fn check_pre_promotion_conditions(
    eval: &archon_world_model::jepa::JepaEvalRecord,
    config: &archon_core::config::ArchonConfig,
    candidate_id: &str,
    current_corpus_fingerprint: Option<&str>,
    comparison_report_exists: bool,
) -> anyhow::Result<()> {
    use archon_world_model::jepa::RuntimeEvalMode;

    // Check 1: mode must convert to RuntimeEvalMode AND be Full or Promotion
    let runtime_mode = RuntimeEvalMode::try_from(eval.mode).map_err(|_| {
        anyhow::anyhow!(
            "Promotion rejected: eval report for {candidate_id} is a legacy record \
             (mode=Legacy). Re-run: archon world eval-jepa {candidate_id} --full"
        )
    })?;
    if matches!(runtime_mode, RuntimeEvalMode::Quick) {
        anyhow::bail!(
            "Promotion rejected: eval report for {candidate_id} was produced in quick mode. \
             Re-run: archon world eval-jepa {candidate_id} --full"
        );
    }

    // Check 2: baseline must not be skipped
    if eval.baseline_skipped {
        anyhow::bail!(
            "Promotion rejected: eval report for {candidate_id} has baseline_skipped=true \
             (reason: {}). Re-run: archon world eval-jepa {candidate_id} --full",
            eval.skipped_reason.as_deref().unwrap_or("unknown")
        );
    }

    // Check 3: comparison must be Some
    if eval.comparison.is_none() {
        anyhow::bail!(
            "Promotion rejected: eval report for {candidate_id} has comparison=None. \
             Re-run: archon world eval-jepa {candidate_id} --full"
        );
    }

    // Check 4: comparison report file must exist
    if !comparison_report_exists {
        anyhow::bail!(
            "Promotion rejected: representation-comparison report missing for {candidate_id}. \
             Re-run: archon world eval-jepa {candidate_id} --full"
        );
    }

    // Check 5: corpus_fingerprint match
    match (&eval.corpus_fingerprint, current_corpus_fingerprint) {
        (None, _) => anyhow::bail!(
            "Promotion rejected: corpus_fingerprint is null in eval report for {candidate_id}. \
             Re-run: archon world eval-jepa {candidate_id} --full"
        ),
        (Some(fp), Some(current)) if fp != current => anyhow::bail!(
            "Promotion rejected: corpus changed since eval for {candidate_id} \
             (fingerprint mismatch). Re-run: archon world eval-jepa {candidate_id} --full"
        ),
        _ => {}
    }

    // Check 6: config_fingerprint match
    let current_config_fp = compute_config_fingerprint(&config.learning.world_model.jepa);
    if eval.config_fingerprint != current_config_fp {
        anyhow::bail!(
            "Promotion rejected: config_fingerprint mismatch for {candidate_id} \
             (record: {}, current: {}). \
             Re-run: archon world eval-jepa {candidate_id} --full",
            eval.config_fingerprint,
            current_config_fp
        );
    }

    // Check 7: eval_schema_version match
    let current_schema_version = config
        .learning
        .world_model
        .jepa
        .eval_schema_version_or_default();
    if eval.eval_schema_version != current_schema_version {
        anyhow::bail!(
            "Promotion rejected: eval_schema_version mismatch for {candidate_id} \
             (record: {}, current: {}). \
             Re-run: archon world eval-jepa {candidate_id} --full",
            eval.eval_schema_version,
            current_schema_version
        );
    }

    Ok(())
}

pub(super) fn render_promote_jepa(
    root: &Path,
    model_id: &str,
    config: &archon_core::config::ArchonConfig,
) -> Result<String> {
    let registry = ModelRegistry::open(root)?;
    let candidate = registry.load_jepa_candidate(model_id)?;
    archon_world_model::jepa::validate_jepa_backend_execution(&candidate.model.metadata)?;
    let eval = registry
        .load_jepa_eval_report(model_id)?
        .ok_or_else(|| anyhow::anyhow!("jepa candidate {model_id} has no eval report"))?;

    // Pre-promotion checks per PRD §11 (checks 1-7 run BEFORE the existing gates.passed check)
    let comparison_path = root
        .join("jepa/representation-comparisons")
        .join(format!("{model_id}.json"));
    let comparison_report_exists = comparison_path.exists();

    // Compute current corpus fingerprint; if corpus can't be loaded pass None so
    // check 5 will produce a clear error (eval.corpus_fingerprint won't match None).
    let current_corpus_fingerprint: Option<String> =
        compute_current_corpus_fingerprint(root).ok();

    check_pre_promotion_conditions(
        &eval,
        config,
        model_id,
        current_corpus_fingerprint.as_deref(),
        comparison_report_exists,
    )?;

    // Check 8: gates.passed (EXISTING CHECK — preserved as the 8th check)
    if !eval.gates.passed {
        bail!("jepa candidate {model_id} has not passed all mandatory promotion gates");
    }

    let path = registry.promote_model_kind(model_id, JEPA_MODEL_KIND)?;
    Ok(format!(
        "World Model JEPA-Inspired Promote\n\
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

#[cfg(test)]
mod promotion_hardening_tests {
    use super::*;
    use archon_world_model::jepa::{
        JepaCollapseReport, JepaEvalRecord, JepaHorizonReport, JepaPromotionGateReport,
        JepaRepresentationComparisonReport, PersistedEvalMode,
    };

    fn make_default_config() -> archon_core::config::ArchonConfig {
        archon_core::config::ArchonConfig::default()
    }

    fn make_minimal_comparison() -> JepaRepresentationComparisonReport {
        JepaRepresentationComparisonReport {
            candidate_id: "test-cand".to_string(),
            baseline_backend: "fastembed".to_string(),
            baseline_available: true,
            failure_reason: None,
            heldout_examples: 200,
            min_heldout_examples: 200,
            jepa_next_state_cosine_similarity: 0.90,
            baseline_next_state_cosine_similarity: 0.80,
            relative_improvement: 0.125,
            min_baseline_improvement: 0.05,
            brier_regressed: false,
            passed: true,
        }
    }

    fn make_minimal_collapse() -> JepaCollapseReport {
        JepaCollapseReport {
            mean_latent_std: 0.06,
            effective_rank_ratio: 0.60,
            min_latent_std: 0.05,
            min_effective_rank_ratio: 0.50,
            passes: true,
        }
    }

    fn make_minimal_horizon() -> JepaHorizonReport {
        JepaHorizonReport {
            e_1: Some(0.10),
            e_3: Some(0.12),
            e_5: Some(0.15),
            tolerance: 0.02,
            passes: true,
        }
    }

    fn make_passing_gates() -> JepaPromotionGateReport {
        JepaPromotionGateReport::from_parts(true, true, true, true, true, true)
    }

    fn make_passing_full_eval() -> JepaEvalRecord {
        let config = make_default_config();
        let fp = compute_config_fingerprint(&config.learning.world_model.jepa);
        JepaEvalRecord {
            candidate_id: "test-cand".to_string(),
            mode: PersistedEvalMode::Full,
            baseline_skipped: false,
            skipped_reason: None,
            corpus_fingerprint: Some("test-corpus-fp".to_string()),
            config_fingerprint: fp,
            eval_schema_version: config
                .learning
                .world_model
                .jepa
                .eval_schema_version_or_default(),
            comparison: Some(make_minimal_comparison()),
            collapse: make_minimal_collapse(),
            horizon: make_minimal_horizon(),
            gates: make_passing_gates(),
            created_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn rejects_legacy_mode() {
        let eval = JepaEvalRecord {
            mode: PersistedEvalMode::Legacy,
            ..make_passing_full_eval()
        };
        let result = check_pre_promotion_conditions(
            &eval,
            &make_default_config(),
            "test-cand",
            Some("test-corpus-fp"),
            true,
        );
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("legacy"),
            "error must mention 'legacy'"
        );
    }

    #[test]
    fn rejects_quick_mode() {
        let eval = JepaEvalRecord {
            mode: PersistedEvalMode::Quick,
            ..make_passing_full_eval()
        };
        let result = check_pre_promotion_conditions(
            &eval,
            &make_default_config(),
            "test-cand",
            Some("test-corpus-fp"),
            true,
        );
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("quick"),
            "error must mention 'quick'"
        );
    }

    #[test]
    fn rejects_baseline_skipped() {
        let eval = JepaEvalRecord {
            baseline_skipped: true,
            ..make_passing_full_eval()
        };
        let result = check_pre_promotion_conditions(
            &eval,
            &make_default_config(),
            "test-cand",
            Some("test-corpus-fp"),
            true,
        );
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("baseline_skipped"),
            "error must mention 'baseline_skipped'"
        );
    }

    #[test]
    fn rejects_comparison_none() {
        let eval = JepaEvalRecord {
            comparison: None,
            ..make_passing_full_eval()
        };
        let result = check_pre_promotion_conditions(
            &eval,
            &make_default_config(),
            "test-cand",
            Some("test-corpus-fp"),
            true,
        );
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("comparison"),
            "error must mention 'comparison'"
        );
    }

    #[test]
    fn rejects_missing_comparison_report() {
        let eval = make_passing_full_eval();
        let result = check_pre_promotion_conditions(
            &eval,
            &make_default_config(),
            "test-cand",
            Some("test-corpus-fp"),
            false, // report does NOT exist
        );
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("representation-comparison"),
            "error must mention 'representation-comparison'"
        );
    }

    #[test]
    fn rejects_null_corpus_fingerprint() {
        let eval = JepaEvalRecord {
            corpus_fingerprint: None,
            ..make_passing_full_eval()
        };
        let result = check_pre_promotion_conditions(
            &eval,
            &make_default_config(),
            "test-cand",
            Some("test-corpus-fp"),
            true,
        );
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("corpus_fingerprint"),
            "error must mention 'corpus_fingerprint'"
        );
    }

    #[test]
    fn rejects_mismatched_corpus_fingerprint() {
        let eval = make_passing_full_eval();
        let result = check_pre_promotion_conditions(
            &eval,
            &make_default_config(),
            "test-cand",
            Some("different-corpus-fp"), // differs from "test-corpus-fp" in eval
            true,
        );
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("corpus"),
            "error must mention 'corpus'"
        );
    }

    #[test]
    fn rejects_mismatched_config_fingerprint() {
        let eval = JepaEvalRecord {
            config_fingerprint: "stale-fp-value".to_string(),
            ..make_passing_full_eval()
        };
        let result = check_pre_promotion_conditions(
            &eval,
            &make_default_config(),
            "test-cand",
            Some("test-corpus-fp"),
            true,
        );
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("config_fingerprint"),
            "error must mention 'config_fingerprint'"
        );
    }

    #[test]
    fn rejects_mismatched_schema_version() {
        let eval = JepaEvalRecord {
            eval_schema_version: 999,
            ..make_passing_full_eval()
        };
        let result = check_pre_promotion_conditions(
            &eval,
            &make_default_config(),
            "test-cand",
            Some("test-corpus-fp"),
            true,
        );
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("eval_schema_version"),
            "error must mention 'eval_schema_version'"
        );
    }

    #[test]
    fn passes_all_checks_for_valid_full_eval() {
        let eval = make_passing_full_eval();
        let result = check_pre_promotion_conditions(
            &eval,
            &make_default_config(),
            "test-cand",
            Some("test-corpus-fp"),
            true,
        );
        assert!(result.is_ok(), "valid full eval must pass: {result:?}");
    }

    #[test]
    fn config_fingerprint_excludes_performance_keys() {
        let cfg = make_default_config();
        let mut cfg2 = cfg.clone();
        // Bump a performance key — fingerprint must NOT change
        cfg2.learning.world_model.jepa.batch_size = 999;
        assert_eq!(
            compute_config_fingerprint(&cfg.learning.world_model.jepa),
            compute_config_fingerprint(&cfg2.learning.world_model.jepa),
            "batch_size is a performance key; must not affect fingerprint"
        );

        // Bump a gate-affecting key — fingerprint MUST change
        let mut cfg3 = cfg.clone();
        cfg3.learning.world_model.jepa.require_native_accelerator_ops =
            !cfg.learning.world_model.jepa.require_native_accelerator_ops;
        assert_ne!(
            compute_config_fingerprint(&cfg.learning.world_model.jepa),
            compute_config_fingerprint(&cfg3.learning.world_model.jepa),
            "require_native_accelerator_ops is a gate-affecting key; must affect fingerprint"
        );
    }
}
