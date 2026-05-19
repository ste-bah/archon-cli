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
