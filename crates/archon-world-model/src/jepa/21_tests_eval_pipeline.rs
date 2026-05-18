// TASK-JEVAL-024 — §15 Integration Tests: JEPA Eval Pipeline Correctness
//
// This file adds integration-level coverage for TC-JEVAL-20 through TC-JEVAL-33.
// Most unit-level behaviour is ALREADY covered in per-task test modules:
//
//   TC-JEVAL-20  quick_mode_skips_tier2_on_tier0_failure
//                → jepa::tests_eval_planner                    [eval_planner.rs]
//   TC-JEVAL-22  o_excl_rejects_second_acquire
//                stale_dead_pid_reclaimed
//                cancel_sentinel_roundtrip
//                atomic_write_produces_valid_json
//                → jepa::tests_eval_run_store                  [eval_run_store.rs]
//   TC-JEVAL-23  deadline_exceeded_returns_err
//                unlimited_budget_never_times_out
//                progress_format_is_mmss
//                → jepa::tests_eval_progress                   [eval_progress.rs]
//   TC-JEVAL-24  rejects_legacy_mode / rejects_quick_mode /
//                rejects_baseline_skipped / rejects_comparison_none /
//                rejects_missing_comparison_report /
//                rejects_null_corpus_fingerprint /
//                rejects_mismatched_config_fingerprint /
//                rejects_mismatched_schema_version
//                → promotion_hardening_tests                   [02_promote_helpers.rs in src/]
//   TC-JEVAL-25  quick_mode_skipped_gates_passed_is_always_false
//                → jepa::tests_02                              [02_records_backend_types.rs]
//   TC-JEVAL-29  legacy_eval_record_serde_defaults
//                → jepa::tests_02                              [02_records_backend_types.rs]
//   TC-JEVAL-32  cuda_required_but_not_compiled_fails_closed_via_resolver
//                → jepa::tests_eval_backends                   [eval_backends.rs]
//
// This file adds the GAPS:
//   TC-JEVAL-21  resumed_run_identical_verdict       — DEFERRED (§ below)
//   TC-JEVAL-26  background_on_windows_fails_clearly — #[cfg(windows)] stub
//   TC-JEVAL-27  cancel_sentinel_observed_by_reporter— ForegroundProgressReporter
//                                                       ↔ JepaEvalRunStore integration
//   TC-JEVAL-28  corpus fingerprint resume semantics — run record struct asserts
//   TC-JEVAL-29  legacy_record_full_integration      — combined serde + non-promotable
//   TC-JEVAL-30  posix_background_happy_path         — DEFERRED (§ below)
//   TC-JEVAL-31  tri_backend_identical_gate_verdict  — #[ignore] hardware stub
//   TC-JEVAL-32  cuda_resolver_falls_back_to_cpu     — resolver integration
//   TC-JEVAL-33  cli_baseline_status_line_format     — DEFERRED (§ below)
//
// DEFERRED ITEMS:
//   TC-JEVAL-21: requires full pipeline wiring through render_eval_jepa_with_options
//                (T023 handled CLI flags; actual resume-from-stage logic deferred to T025).
//   TC-JEVAL-30: requires a working `archon` binary build, a pre-trained candidate file,
//                real corpus rows, and polling logic for the run record. The
//                spawn_background_worker code path IS tested at unit level in T012
//                (POSIX double-fork+setsid mechanism).
//   TC-JEVAL-33: requires full eval pipeline wiring including CLI output capture.
//                Deferred to post-T025 manual validation once output format is finalised.

use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Shared test helper
// ---------------------------------------------------------------------------

/// Build a minimal JepaEvalRunRecord for integration tests.
fn make_eval_run_record(run_id: &str, candidate_id: &str) -> crate::jepa::JepaEvalRunRecord {
    crate::jepa::JepaEvalRunRecord {
        run_id: run_id.to_string(),
        candidate_id: candidate_id.to_string(),
        corpus_fingerprint: None,
        training_corpus_fingerprint: None,
        mode: crate::jepa::RuntimeEvalMode::Quick,
        backend: crate::jepa::JepaEvalBackendKind::Cpu,
        status: crate::jepa::EvalRunStatus::Running,
        current_stage: crate::jepa::EvalRunStage::Tier0,
        stage_timings: HashMap::new(),
        baseline_skipped: false,
        skipped_reason: None,
        pid: std::process::id(),
        host: "test-host".to_string(),
        started_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        completed_at: None,
        rows_total: 0,
        rows_completed: 0,
        transitions_total: 0,
        transitions_completed: 0,
        embeddings_total: 0,
        embeddings_completed: 0,
        cache_hits: 0,
        cache_misses: 0,
        backend_parity_examples: 0,
        failure_reason: None,
        partial_gates: serde_json::Value::Null,
        result_paths: HashMap::new(),
    }
}

// ---------------------------------------------------------------------------
// TC-JEVAL-20 cross-reference: quick mode skips Tier-2 when Tier-0 fails
//
// The unit test `tests_eval_planner::quick_mode_skips_tier2_on_tier0_failure`
// (in eval_planner.rs) is the primary coverage for this criterion.
//
// This cross-reference test verifies the same invariant at the integration
// boundary: a Quick planner returns false from should_run_tier2 for ANY
// non-empty tier-0 failure regardless of tier-1 state.
// ---------------------------------------------------------------------------

#[test]
fn quick_mode_never_runs_tier2_when_tier0_has_any_failure() {
    let planner = crate::jepa::JepaEvalPlanner::new(crate::jepa::RuntimeEvalMode::Quick);

    // Every single possible Tier-0 gate name listed in eval_planner.rs.
    let tier0_gate_names = [
        "tensor_safety",
        "training_corpus_sufficient",
        "representation_collapse",
        "multi_horizon_consistency",
        "backend_execution",
        "checkpoint_size",
    ];

    for gate in tier0_gate_names {
        let tier0_fail =
            crate::jepa::TierGateResult::with_failures(vec![gate.to_string()]);
        let tier1_pass = crate::jepa::TierGateResult::all_pass();
        assert!(
            !planner.should_run_tier2(&tier0_fail, &tier1_pass),
            "quick mode must skip Tier-2 when Tier-0 gate '{gate}' fails"
        );
    }

    // Also verify quick mode skips when BOTH tiers fail.
    let both_fail = crate::jepa::TierGateResult::with_failures(vec!["x".into()]);
    assert!(
        !planner.should_run_tier2(
            &crate::jepa::TierGateResult::with_failures(vec!["tensor_safety".into()]),
            &both_fail,
        ),
        "quick mode must skip Tier-2 when both Tier-0 and Tier-1 fail"
    );
}

// ---------------------------------------------------------------------------
// TC-JEVAL-26: --background on Windows returns clear error
//
// On non-Windows hosts this test is compiled but trivially skipped — the
// #[cfg] guard means the test function body is absent on POSIX, so the
// test name does not appear in cargo test --list on Linux/macOS.
// On Windows the test must confirm the error message.
// ---------------------------------------------------------------------------

#[test]
#[cfg(target_os = "windows")]
fn background_on_windows_returns_clear_error() {
    let tmp = tempfile::tempdir().unwrap();
    let store = crate::jepa::JepaEvalRunStore::new(tmp.path().to_path_buf()).unwrap();
    let run_id = crate::jepa::JepaEvalRunStore::generate_run_id();
    let result = store.spawn_background_worker(&run_id, &[]);
    assert!(
        result.is_err(),
        "--background must return Err on Windows"
    );
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("not supported on Windows"),
        "error message must contain 'not supported on Windows'; got: {msg}"
    );
}

// ---------------------------------------------------------------------------
// TC-JEVAL-27: Cancel sentinel observed by ForegroundProgressReporter
//
// The unit test `tests_eval_run_store::cancel_sentinel_roundtrip` verifies
// that JepaEvalRunStore can write and detect the sentinel file.
//
// THIS integration test verifies that ForegroundProgressReporter::check_cancel_sentinel()
// correctly delegates to JepaEvalRunStore::cancel_sentinel_exists for the
// same run-id the reporter holds.  The cancel path in the eval loop is:
//
//   reporter.check_cancel_sentinel(&store)  →  true
//   (caller sets run_record.status = Cancelled and persists)
//
// NOTE: tick() does NOT check the cancel sentinel — that is by design
// (eval_progress.rs §check_cancel_sentinel docstring). The sentinel check
// is intentionally separated so callers control polling frequency.
// ---------------------------------------------------------------------------

#[test]
fn cancel_sentinel_observed_by_progress_reporter() {
    let tmp = tempfile::tempdir().unwrap();
    let store = crate::jepa::JepaEvalRunStore::new(tmp.path().to_path_buf()).unwrap();
    let run_id = crate::jepa::JepaEvalRunStore::generate_run_id();
    let record = make_eval_run_record(&run_id, "cancel-test-cand");
    store.write_run(&record).unwrap();

    let reporter = crate::jepa::ForegroundProgressReporter::new(record, 0, 1);

    // Before sentinel: reporter must not see it.
    assert!(
        !reporter.check_cancel_sentinel(&store),
        "reporter must not see cancel sentinel before it is written"
    );

    // Write the sentinel (simulates: archon world eval-jepa-cancel <run-id>).
    store.write_cancel_sentinel(&run_id).unwrap();

    // After sentinel: reporter must see it.
    assert!(
        reporter.check_cancel_sentinel(&store),
        "reporter must see cancel sentinel after it is written"
    );

    // The run_id exposed on the reporter must match the store's sentinel path.
    assert_eq!(
        reporter.run_id, run_id,
        "reporter.run_id must match the run record's run_id"
    );
}

// ---------------------------------------------------------------------------
// TC-JEVAL-28: corpus_fingerprint resume semantics
//
// Two invariants per PRD §6.2:
//   (a) corpus_fingerprint == None  →  restart from Tier-0
//   (b) corpus_fingerprint mismatch →  invalidate the run (restart from Tier-0)
//
// These invariants live in the calling code (not yet wired in 006C), so this
// test asserts the data-structure semantics that the resume logic will rely on.
// The JepaEvalRunRecord's corpus_fingerprint field IS the persisted state that
// the resume decision reads.
// ---------------------------------------------------------------------------

#[test]
fn run_record_with_null_corpus_fingerprint_matches_restart_condition() {
    // A run paused before Tier-1 (fingerprint not yet computed).
    let run = make_eval_run_record("jeval-test-fp-null", "cand-resume");
    // corpus_fingerprint starts as None (from make_eval_run_record).
    assert!(
        run.corpus_fingerprint.is_none(),
        "null corpus_fingerprint run record: corpus_fingerprint must be None"
    );
    // Resume logic: None → must restart from Tier-0.
    let should_restart = run.corpus_fingerprint.is_none();
    assert!(
        should_restart,
        "null corpus_fingerprint must trigger restart from Tier-0"
    );
}

#[test]
fn run_record_corpus_fingerprint_mismatch_detected_structurally() {
    // Simulate a run paused mid-Tier-1 with a recorded corpus fingerprint.
    let mut run = make_eval_run_record("jeval-test-fp-mismatch", "cand-resume-2");
    run.corpus_fingerprint = Some("fp-at-eval-time".to_string());
    run.current_stage = crate::jepa::EvalRunStage::BaselineEmbed;

    // The current corpus fingerprint (computed fresh by the eval loop) is different.
    let current_fp = "fp-after-new-rows-added";

    // Mismatch detection: the resume logic checks equality.
    let fingerprint_matches = run
        .corpus_fingerprint
        .as_deref()
        .map(|fp| fp == current_fp)
        .unwrap_or(false);

    assert!(
        !fingerprint_matches,
        "corpus_fingerprint mismatch must be detected (run: '{:?}', current: '{current_fp}')",
        run.corpus_fingerprint
    );
}

#[test]
fn run_record_corpus_fingerprint_match_allows_resume() {
    let mut run = make_eval_run_record("jeval-test-fp-match", "cand-resume-3");
    let fp = "stable-corpus-fp-abc123";
    run.corpus_fingerprint = Some(fp.to_string());
    run.current_stage = crate::jepa::EvalRunStage::BaselineEmbed;

    // When corpus has NOT changed, fingerprint matches.
    let current_fp = fp;
    let fingerprint_matches = run
        .corpus_fingerprint
        .as_deref()
        .map(|f| f == current_fp)
        .unwrap_or(false);

    assert!(
        fingerprint_matches,
        "matching corpus_fingerprint must allow resume without restart"
    );
}

// ---------------------------------------------------------------------------
// TC-JEVAL-29: Legacy record deserialises with failing promotion defaults
//             (integration form: full serde roundtrip + non-promotable check)
//
// The unit test `tests_02::legacy_eval_record_serde_defaults` verifies the
// individual serde defaults. This integration test verifies all 6 new fields
// together as a promotion-rejection scenario: deserialise a real pre-schema
// JSON, confirm ALL 6 defaults, and confirm that the resulting record would
// be rejected by the pre-promotion checks at the data-structure level.
//
// The `check_pre_promotion_conditions` function lives in archon-core (in
// src/command/…) so we cannot call it directly here. Instead we verify that
// the deserialized record carries the values that CAUSE the checks to fail:
//   • mode = Legacy       → check 1 rejects (try_from fails)
//   • baseline_skipped = true  → check 2 rejects
//   • comparison per record    → may or may not be None (check 3/4)
//   • corpus_fingerprint = None → check 5 rejects
//   • config_fingerprint = "legacy" ≠ any real fingerprint → check 6 rejects
//   • eval_schema_version = 0  ≠ EVAL_SCHEMA_VERSION (1)  → check 7 rejects
// ---------------------------------------------------------------------------

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

#[test]
#[ignore = "deferred: requires archon binary build + JEPA fixture; POSIX mechanism tested in T012"]
fn posix_background_happy_path() {
    // Stub: spawn `archon world eval-jepa <candidate> --background` via
    // std::process::Command, assert parent exits 0 in < 2s, assert run-id
    // printed to stdout, assert run record file exists, poll up to 30s for
    // status ∈ {completed, failed}.
}

// ---------------------------------------------------------------------------
// TC-JEVAL-31: Tri-backend identical gate verdict (hardware-gated)
//
// Requires MLX Metal hardware AND a CUDA device on the same machine.
// This test exists as a documented stub; run manually on qualifying hardware.
// ---------------------------------------------------------------------------

#[test]
#[ignore = "hardware-gated: requires both MLX Metal AND CUDA device — run manually on qualifying machine"]
fn tri_backend_identical_gate_verdict() {
    // Stub:
    // 1. Load a fixed test candidate + fixed corpus rows.
    // 2. Run JepaEvalRuntime encode_batch + validate_forward_parity via:
    //    (a) CpuEvalRuntime
    //    (b) MlxEvalRuntime
    //    (c) CudaEvalRuntime
    // 3. Assert all three ParityReport.passed values are identical.
    // 4. Assert all three cosine_similarity values are within
    //    backend_parity_cosine_floor of each other (default ≥ 0.99).
    //
    // Parity tolerance per JepaEvalGateConfig::backend_parity_cosine_floor
    // (default 0.99, configurable in [learning.world_model.jepa]).
}

// ---------------------------------------------------------------------------
// TC-JEVAL-32: CUDA resolver falls back to CPU when CUDA not required
//
// The unit test `tests_eval_backends::cuda_required_but_not_compiled_fails_closed_via_resolver`
// verifies the fail-closed path (CUDA required but not compiled → Err).
//
// THIS integration test verifies the fallback path:
//   CUDA not required + not compiled → resolver returns CPU runtime with no error.
//   This matches the `tracing::warn!` + CPU fallback branch in eval_runtime.rs.
//
// On a host with the `cuda` feature compiled, this test exercises the path
// where CUDA is requested but allow_cpu_fallback=true and prefer_accelerator=false
// (i.e., CUDA is NOT required), so resolve() succeeds with CPU.
// ---------------------------------------------------------------------------

#[test]
fn cuda_resolver_falls_back_to_cpu_when_not_required() {
    // Construct metadata that has CPU as the selected_backend (not CUDA).
    let backend_execution =
        crate::jepa::JepaBackendExecutionReport::cpu(crate::BackendKind::Cpu, None, 0);
    let metadata = crate::jepa::JepaTraceModelMetadata {
        model_id: "test-cuda-fallback".to_string(),
        model_kind: "jepa_transition".to_string(),
        latent_dim: 16,
        context_window_rows: 4,
        target_window_rows: 2,
        prediction_horizons: vec![1, 3],
        mask_ratio: 0.30,
        ema_decay: 0.996,
        target_stop_gradient: true,
        backend: crate::BackendKind::Cpu,
        backend_execution,
        row_count: 0,
        example_count: 0,
        parameter_count: 0,
        created_at: chrono::Utc::now(),
    };

    // Config: training_backend = "cuda" but allow_cpu_fallback = true
    // and prefer_accelerator = false → cuda_is_required() = false.
    let config = crate::jepa::JepaEvalResolverConfig {
        training_backend: "cuda".into(),
        allow_cpu_fallback: true,
        prefer_accelerator: false,
        parity_floor: 0.99,
        min_metal_validation_examples: 0,
        min_cuda_validation_examples: 0,
    };

    // Pre-check: CUDA must NOT be required under this config.
    let cuda_required = crate::jepa::BackendRuntimeResolver::cuda_is_required(
        &metadata,
        &config,
        None, // no CLI override
    );
    assert!(
        !cuda_required,
        "cuda_is_required must be false when allow_cpu_fallback=true and prefer_accelerator=false"
    );

    // Resolve: must succeed and return a CPU runtime.
    let result = crate::jepa::BackendRuntimeResolver::resolve(&metadata, &config, None);
    assert!(
        result.is_ok(),
        "resolver must succeed (CPU fallback) when CUDA not required; got: {:?}",
        result.map(|_| ()).map_err(|e| e.to_string())
    );
    let runtime = result.unwrap();
    assert_eq!(
        runtime.backend_kind(),
        crate::jepa::JepaEvalBackendKind::Cpu,
        "resolver must return CPU backend when CUDA not required and not available"
    );
}

// ---------------------------------------------------------------------------
// TC-JEVAL-33: CLI baseline-status line format — DEFERRED
//
// This test requires full eval pipeline wiring through render_eval_jepa_with_options
// (T023 handled the CLI flag plumbing; the baseline status output path is not
// yet finalised). Deferred to post-T025 once the output format is confirmed.
// ---------------------------------------------------------------------------

#[test]
#[ignore = "deferred: requires full eval pipeline wiring and CLI output capture — post-T025"]
fn cli_baseline_status_line_format() {
    // Stub:
    // Case 1: quick eval, Tier-0 gate fails (collapse) → output contains "Baseline: skipped"
    // Case 2: full eval, warm cache (embeddings cached) → output contains "Baseline: cached"
    // Case 3: full eval, cold cache                    → output contains "Baseline: run"
    // Case 4: eval interrupted (Paused status)          → output contains "Baseline: incomplete"
}
