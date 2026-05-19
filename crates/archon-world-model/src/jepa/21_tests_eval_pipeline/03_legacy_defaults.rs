#[test]
fn legacy_record_deserialises_with_all_failing_promotion_defaults() {
    // This JSON simulates a pre-schema on-disk eval record that is missing all
    // 6 new fields (mode, baseline_skipped, corpus_fingerprint,
    // config_fingerprint, eval_schema_version, skipped_reason).
    let legacy_json = serde_json::json!({
        "candidate_id": "legacy-candidate-abc",
        // no "mode" field
        // no "baseline_skipped" field
        // no "corpus_fingerprint" field
        // no "config_fingerprint" field
        // no "eval_schema_version" field
        "comparison": {
            "candidate_id": "legacy-candidate-abc",
            "baseline_backend": "fastembed",
            "baseline_available": true,
            "failure_reason": null,
            "heldout_examples": 210,
            "min_heldout_examples": 200,
            "jepa_next_state_cosine_similarity": 0.88,
            "baseline_next_state_cosine_similarity": 0.80,
            "relative_improvement": 0.10,
            "min_baseline_improvement": 0.05,
            "brier_regressed": false,
            "passed": true
        },
        "collapse": {
            "mean_latent_std": 0.06,
            "effective_rank_ratio": 0.55,
            "min_latent_std": 0.05,
            "min_effective_rank_ratio": 0.50,
            "passes": true
        },
        "horizon": {
            "e_1": 0.10,
            "e_3": 0.13,
            "e_5": 0.16,
            "tolerance": 0.02,
            "passes": true
        },
        "gates": {
            "corpus_sufficient": true,
            "representation_baseline": true,
            "representation_collapse": true,
            "multi_horizon_consistency": true,
            "checkpoint_size": true,
            "tensor_safety": true,
            "backend_execution": true,
            "passed": true
        },
        "created_at": "2025-01-01T00:00:00Z"
    });

    let record: crate::jepa::JepaEvalRecord =
        serde_json::from_value(legacy_json).expect("legacy record must deserialise");

    // === Verify all 6 fields default to fail-promotion values ===

    // Field 1: mode defaults to Legacy (serde(other) on PersistedEvalMode).
    assert_eq!(
        record.mode,
        crate::jepa::PersistedEvalMode::Legacy,
        "legacy record: mode must default to Legacy"
    );

    // Field 2: baseline_skipped defaults to true (default_baseline_skipped fn).
    assert!(
        record.baseline_skipped,
        "legacy record: baseline_skipped must default to true"
    );

    // Field 3: corpus_fingerprint defaults to None.
    assert!(
        record.corpus_fingerprint.is_none(),
        "legacy record: corpus_fingerprint must default to None"
    );

    // Field 4: config_fingerprint defaults to "legacy".
    assert_eq!(
        record.config_fingerprint, "legacy",
        "legacy record: config_fingerprint must default to 'legacy'"
    );

    // Field 5: eval_schema_version defaults to 0 (not current EVAL_SCHEMA_VERSION=1).
    assert_eq!(
        record.eval_schema_version, 0,
        "legacy record: eval_schema_version must default to 0"
    );
    // Cross-check that 0 ≠ current schema version.
    assert_ne!(
        record.eval_schema_version,
        crate::jepa::EVAL_SCHEMA_VERSION,
        "eval_schema_version 0 must differ from EVAL_SCHEMA_VERSION={} so that the \
         schema-version pre-promotion check rejects the record",
        crate::jepa::EVAL_SCHEMA_VERSION
    );

    // Field 6: skipped_reason not required but should be None for legacy records.
    // (This field defaults to None via Option<String>.)
    assert!(
        record.skipped_reason.is_none(),
        "legacy record: skipped_reason must default to None"
    );

    // === Verify promotion-rejection can be diagnosed from the record fields ===

    // Check 1 trigger: Legacy cannot be converted to RuntimeEvalMode.
    let runtime_mode_result = crate::jepa::RuntimeEvalMode::try_from(record.mode);
    assert!(
        runtime_mode_result.is_err(),
        "Legacy mode must not convert to RuntimeEvalMode (triggers pre-promotion check 1)"
    );
    assert!(
        runtime_mode_result.unwrap_err().to_string().contains("legacy"),
        "conversion error must mention 'legacy'"
    );

    // Check 2 trigger: baseline_skipped = true.
    assert!(
        record.baseline_skipped,
        "baseline_skipped=true triggers pre-promotion check 2"
    );

    // Check 5 trigger: corpus_fingerprint = None.
    assert!(
        record.corpus_fingerprint.is_none(),
        "corpus_fingerprint=None triggers pre-promotion check 5"
    );

    // Check 6 trigger: config_fingerprint = "legacy" ≠ any real SHA-256 fingerprint.
    assert_eq!(
        record.config_fingerprint.as_str(),
        "legacy",
        "config_fingerprint='legacy' triggers pre-promotion check 6"
    );
    assert!(
        record.config_fingerprint.len() < 64,
        "config_fingerprint='legacy' is too short to be a valid SHA-256 hex; triggers check 6"
    );

    // Check 7 trigger: eval_schema_version = 0 ≠ 1.
    assert_ne!(
        record.eval_schema_version,
        crate::jepa::EVAL_SCHEMA_VERSION,
        "eval_schema_version=0 triggers pre-promotion check 7"
    );
}

// ---------------------------------------------------------------------------
// TC-JEVAL-30: POSIX background happy path — DEFERRED
//
// This test requires:
//   - A working `archon` binary build accessible from std::process::Command
//   - A pre-trained JEPA candidate file in the test fixture directory
//   - Real corpus rows in the world-model store
//   - Polling logic for the on-disk run record
//
// The spawn_background_worker code path IS tested at unit level in T012:
//   tests_eval_run_store::cancel_sentinel_roundtrip (store I/O)
//   and the POSIX double-fork+setsid mechanism is confirmed to compile.
//
// Deferred until a full eval fixture set is available post-T025.
// ---------------------------------------------------------------------------
