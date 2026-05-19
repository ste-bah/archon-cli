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
