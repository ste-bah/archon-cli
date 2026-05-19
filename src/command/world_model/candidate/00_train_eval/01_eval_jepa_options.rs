/// Entry point for `archon world eval-jepa` with the v1.3.2 eval flags.
/// TASK-JEVAL-023; CRIT-4 Tier-0 quick-mode short-circuit wired here.
///
/// Default (no --full): builds JepaEvalGateConfig from config, loads the
/// candidate, runs Tier-0 gates. If any Tier-0 gate fails, writes a
/// `baseline_skipped: true` eval record and returns WITHOUT running fastembed.
/// This is the behaviour advertised by the migration notice (DEC-JEVAL-01).
///
/// --full: skip Tier-0 short-circuit and delegate directly to render_eval_jepa.
///
/// Full runtime backend override wiring and the --no-cache write-bypass remain
/// staged follow-ups; this handler emits visible warnings instead of silently
/// pretending those flags affect the full eval path.
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
