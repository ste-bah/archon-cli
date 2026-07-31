use super::*;

/// Seed a corpus that survives verified-label materialisation.
///
/// Since 1d157a20 training and evaluation read `load_verified_training_rows()`,
/// which discards any row whose success label cannot be proven from guardrail
/// evidence. Rows alone therefore materialise `success: None`, which leaves no
/// positives, no Brier improvement, and a failing promotion gate.
///
/// So each row is joined by `action_attempt_id` to a guarded action carrying a
/// `required_for_final` verification requirement, plus an outcome and a
/// verification result that together prove the claim. The seeded corpus mixes
/// verified successes with a verified failure so the Brier comparison has both
/// classes to work with.
fn seed_training_rows(root: &std::path::Path) {
    use archon_world_model::guardrail::{
        GuardedActionKind, GuardrailFinalStatus, RuntimeTaskClass, VerificationKind,
        VerificationOutcome, VerificationRequirement, VerificationStatus, WorldGuardedAction,
        WorldGuardrailOutcome, append_guarded_action, append_guardrail_outcome,
        append_verification_outcome,
    };
    use archon_world_model::integration::WorldAdvisorSurface;
    use archon_world_model::schema::{WorldActionKind, WorldTraceRow};

    const REQUIREMENT_ID: &str = "required-tests";

    // (attempt id, row kind, excerpt, did the required verification pass)
    let seeds: [(&str, WorldActionKind, &str, bool); 3] = [
        ("attempt-1", WorldActionKind::ToolCall, "run cargo test", true),
        (
            "attempt-2",
            WorldActionKind::Verification,
            "cargo test failed",
            false,
        ),
        (
            "attempt-3",
            WorldActionKind::Retry,
            "fix test and rerun",
            true,
        ),
    ];

    let store = archon_world_model::storage::WorldModelStore::open(root).unwrap();
    let mut rows = Vec::new();

    for (index, (attempt_id, kind, excerpt, passed)) in seeds.into_iter().enumerate() {
        let mut row = WorldTraceRow::new("session-1", kind)
            .with_row_id(format!("r{}", index + 1))
            .with_action_attempt_id(attempt_id);
        row.redacted_excerpt = Some(excerpt.into());
        rows.push(row);

        let mut action = WorldGuardedAction::new(
            "session-1",
            WorldAdvisorSurface::ToolRun,
            GuardedActionKind::PlanStep,
            "test",
            "test",
        );
        action.action_id = attempt_id.into();
        action.idempotency_key = format!("world_guardrail:action:{attempt_id}");
        // Without a requirement marked `required_for_final` the materialiser
        // has nothing to prove against and returns `None` rather than a label.
        action.verification_plan = vec![VerificationRequirement {
            requirement_id: REQUIREMENT_ID.into(),
            kind: VerificationKind::UnitTests,
            required_for_final: true,
            ..VerificationRequirement::default()
        }];
        append_guarded_action(root, &action).unwrap();

        let status = if passed {
            VerificationStatus::Passed
        } else {
            VerificationStatus::Failed
        };
        let verification = VerificationOutcome {
            action_id: attempt_id.into(),
            requirement_id: REQUIREMENT_ID.into(),
            kind: VerificationKind::UnitTests,
            status,
            idempotency_key: format!("verification:{attempt_id}"),
            ..VerificationOutcome::default()
        };
        append_verification_outcome(root, &verification).unwrap();

        let outcome = WorldGuardrailOutcome {
            outcome_id: format!("outcome-{attempt_id}"),
            action_id: attempt_id.into(),
            // No prediction history in this corpus. Naming one would make the
            // materialiser demand a persisted prediction record that does not
            // exist; labels do not need it.
            prediction_id: None,
            task_class: RuntimeTaskClass::CodingChange,
            // Only `CompletedVerified` can yield a positive; a failed
            // verification must present as failed or the two disagree and the
            // materialiser records a contradiction instead of a label.
            final_status: if passed {
                GuardrailFinalStatus::CompletedVerified
            } else {
                GuardrailFinalStatus::BlockedFailedVerification
            },
            verification_outcomes: vec![verification],
            idempotency_key: format!("world_guardrail:outcome:{attempt_id}"),
            ..WorldGuardrailOutcome::default()
        };
        append_guardrail_outcome(root, &outcome).unwrap();
    }

    store.persist_rows(&rows).unwrap();
}

fn candidate_id_from(rendered: &str) -> String {
    rendered
        .lines()
        .find_map(|line| line.trim().strip_prefix("Candidate: "))
        .expect("train output should contain candidate id")
        .to_string()
}

fn passing_jepa_eval_record(candidate_id: &str) -> archon_world_model::jepa::JepaEvalRecord {
    archon_world_model::jepa::JepaEvalRecord {
        candidate_id: candidate_id.into(),
        mode: archon_world_model::jepa::PersistedEvalMode::Full,
        baseline_skipped: false,
        skipped_reason: None,
        corpus_fingerprint: None,
        config_fingerprint: "legacy".to_string(),
        eval_schema_version: 0,
        comparison: Some(archon_world_model::jepa::JepaRepresentationComparisonReport {
            candidate_id: candidate_id.into(),
            baseline_backend: "fastembed".into(),
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
        }),
        collapse: archon_world_model::jepa::JepaCollapseReport {
            mean_latent_std: 0.06,
            effective_rank_ratio: 0.60,
            min_latent_std: 0.05,
            min_effective_rank_ratio: 0.50,
            passes: true,
        },
        horizon: archon_world_model::jepa::JepaHorizonReport {
            e_1: Some(0.10),
            e_3: Some(0.12),
            e_5: Some(0.15),
            tolerance: 0.02,
            passes: true,
        },
        gates: archon_world_model::jepa::JepaPromotionGateReport::from_parts(
            true, true, true, true, true, true,
        ),
        created_at: chrono::Utc::now(),
    }
}

fn prediction_id_from(rendered: &str) -> String {
    rendered
        .lines()
        .find_map(|line| line.trim().strip_prefix("Prediction id: "))
        .expect("prediction output should contain prediction id")
        .to_string()
}

fn test_config() -> archon_core::config::ArchonConfig {
    let mut config = archon_core::config::ArchonConfig::default();
    config.learning.world_model.embeddings.provider = "deterministic-hash".into();
    config
}

fn jepa_test_config() -> archon_core::config::ArchonConfig {
    let mut config = test_config();
    config.learning.world_model.model_kind = "jepa_transition".into();
    config.learning.world_model.state_dim = 8;
    config.learning.world_model.jepa.latent_dim = 8;
    config.learning.world_model.jepa.context_window_rows = 2;
    config.learning.world_model.jepa.target_window_rows = 1;
    config.learning.world_model.jepa.prediction_horizons = vec![1];
    config.learning.world_model.training.backend = "cpu".into();
    config
}
