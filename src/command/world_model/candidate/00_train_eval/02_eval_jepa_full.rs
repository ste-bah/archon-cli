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
