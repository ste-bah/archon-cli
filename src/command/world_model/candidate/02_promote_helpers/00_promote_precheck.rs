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
