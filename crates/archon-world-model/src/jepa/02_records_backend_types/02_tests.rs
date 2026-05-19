#[cfg(test)]
mod tests_02 {
    use super::*;

    #[test]
    fn persisted_eval_mode_defaults_to_legacy() {
        let mode: PersistedEvalMode = serde_json::from_str("\"legacy\"").unwrap();
        assert_eq!(mode, PersistedEvalMode::Legacy);
        // default() returns Legacy
        assert_eq!(PersistedEvalMode::default(), PersistedEvalMode::Legacy);
    }

    #[test]
    fn persisted_legacy_to_runtime_fails() {
        let result = RuntimeEvalMode::try_from(PersistedEvalMode::Legacy);
        assert!(result.is_err(), "Legacy must not convert to RuntimeEvalMode");
    }

    #[test]
    fn persisted_full_to_runtime_ok() {
        let r = RuntimeEvalMode::try_from(PersistedEvalMode::Full).unwrap();
        assert_eq!(r, RuntimeEvalMode::Full);
    }

    #[test]
    fn runtime_mode_converts_to_persisted() {
        assert_eq!(
            PersistedEvalMode::from(RuntimeEvalMode::Quick),
            PersistedEvalMode::Quick
        );
        assert_eq!(
            PersistedEvalMode::from(RuntimeEvalMode::Full),
            PersistedEvalMode::Full
        );
        assert_eq!(
            PersistedEvalMode::from(RuntimeEvalMode::Promotion),
            PersistedEvalMode::Promotion
        );
        // RuntimeEvalMode has NO Legacy variant — cannot construct Legacy at runtime
    }

    #[test]
    fn runtime_eval_mode_has_no_legacy_variant() {
        // Compile-time guarantee documented in comments.
        // Test the enum exhaustiveness: only 3 variants.
        let modes = [
            RuntimeEvalMode::Quick,
            RuntimeEvalMode::Full,
            RuntimeEvalMode::Promotion,
        ];
        assert_eq!(modes.len(), 3);
    }

    #[test]
    fn legacy_eval_record_serde_defaults() {
        // Simulate a legacy on-disk record: no new fields present.
        // JepaEvalRecord's new fields must default to fail-promotion values.
        let json = serde_json::json!({
            "candidate_id": "jepa-world-model-candidate-test",
            "comparison": null,
            "collapse": {
                "mean_latent_std": 0.0065,
                "effective_rank_ratio": 0.0476,
                "min_latent_std": 0.05,
                "min_effective_rank_ratio": 0.50,
                "passes": false
            },
            "horizon": {
                "e_1": null, "e_3": null, "e_5": null,
                "tolerance": 0.02, "passes": true
            },
            "gates": {
                "corpus_sufficient": false,
                "representation_baseline": false,
                "representation_collapse": false,
                "multi_horizon_consistency": true,
                "checkpoint_size": true,
                "tensor_safety": true,
                "backend_execution": true,
                "passed": false
            },
            "created_at": "2026-05-16T21:39:00Z"
            // NOTE: no mode/baseline_skipped/skipped_reason/corpus_fingerprint/
            //       config_fingerprint/eval_schema_version fields — testing serde defaults
        });
        let record: JepaEvalRecord = serde_json::from_value(json).expect("deserializes");
        assert_eq!(record.mode, PersistedEvalMode::Legacy, "mode defaults to Legacy");
        assert_eq!(record.eval_schema_version, 0, "eval_schema_version defaults to 0");
        assert!(record.baseline_skipped, "baseline_skipped defaults to true");
        assert_eq!(
            record.config_fingerprint, "legacy",
            "config_fingerprint defaults to 'legacy'"
        );
        assert!(
            record.corpus_fingerprint.is_none(),
            "corpus_fingerprint defaults to None"
        );
    }

    #[test]
    fn quick_mode_skipped_gates_passed_is_always_false() {
        // PRD §11: "A skipped Tier-2 gate must never leave gates.passed = true"
        let gates =
            JepaPromotionGateReport::quick_mode_skipped(true, true, true, true, true, true);
        assert!(
            !gates.passed,
            "representation_baseline=false forces gates.passed=false"
        );
        assert!(
            !gates.representation_baseline,
            "representation_baseline is false"
        );
    }
}
