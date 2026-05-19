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
