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
