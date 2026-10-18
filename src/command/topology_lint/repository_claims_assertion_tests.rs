//! #443: what a test the body specifies asserts at run time is not a claim
//! that the path is in the repository at the base commit.

use super::*;

/// The test specification from the body that tripped #443, verbatim except
/// for its task-specific names, which are generic here. The three runtime
/// paths are absent from every repository, as they were from the real one.
const RUNTIME_SPEC: &str = r#"## Implementation Contract

Add exactly these four `#[test]` functions (no `serial` needed — each uses its own `tempfile::tempdir()`):

1. `gate_review_lists_every_required_risk_category` — on a fresh lake, **first drive the real producer**: seed a validated native dataset via the public `store_ohlcv` ingest (enough bars to clear the production backtest minimum) and call `run_native_backtest("run-gate", <dataset id>, <version>, <config>, 1.0, GENERATED_AT)`; assert the run wrote `backtests/run-gate/adversarial-review.md`, `backtests/run-gate/report.json`, and the aggregate `backtests/adversarial-review.md`, and that the aggregate's `## Findings` section yields at least one `- X risk: (PASS|FAIL)` line per required category.
2. `done_13_real_run_materializes_review_artifacts` — on a fresh lake, seed the validated native dataset via `store_ohlcv`, run `run_native_backtest("run-real", …, GENERATED_AT)`, and assert: (a) `backtests/run-real/report.json` exists and parses as JSON with a `promotion_eligible` key; (b) `backtests/run-real/adversarial-review.md` exists and contains `run_id: `run-real``; (c) the aggregate `backtests/adversarial-review.md` exists; (d) re-running `write_review_aggregate(GENERATED_AT)` (public via `data_store`) returns the aggregate path and its content still parses under the same `## Findings`/`## Residual Gaps` section contract as (b). This is the producer obligation made checkable at the acceptance-gate level: the adversarial review exists on disk because a real run produced it, not because a test hand-seeded it.
3. `done_14_blocked_state_review_fails_closed` — on a fresh lake, assert `write_blocked_state_review(GENERATED_AT)` returns `Err(DataStoreError::InvalidMetadata(_))`; then seed the blocked state, and assert the written `backtests/adversarial-review.md` carries `- status: `failed``.

## Files Expected to Change

- `crates/x/tests/gates.rs` — exists (206 lines)
"#;

#[test]
fn issue443_the_runtime_test_specification_makes_no_repository_claim() {
    assert!(
        claims(RUNTIME_SPEC)
            .iter()
            .all(|(path, _)| !path.starts_with("backtests/")),
        "{:?}",
        claims(RUNTIME_SPEC)
    );
    let (_temp, project, _tasks, tree) = grounded();
    let found = findings_against(&tree, &project, "TASK-X-443", RUNTIME_SPEC);
    assert!(
        found.iter().all(|finding| !finding.contains("backtests/")),
        "{found:?}"
    );
    // The deliverable observation beside it is still read and checked.
    assert_eq!(
        found.len(),
        1,
        "only the false deliverable observation remains: {found:?}"
    );
    assert!(
        found[0].contains("`crates/x/tests/gates.rs` exists"),
        "{found:?}"
    );
}

#[test]
fn issue443_assertion_clauses_are_runtime_expectations() {
    for text in [
        "The test asserts that `out/report.json` exists.",
        "Run the producer and assert `out/report.json` exists.",
        "Assert: (a) `out/a.json` exists, (b) `out/b.json` exists.",
        "Asserting the written `out/report.json` exists, the test passes.",
        "The test checks that `out/report.json` exists under the temp root.",
        "The test checks `out/report.json` exists.",
        "Verify that `out/report.json` exists after the run.",
        "Assert `out/a.json` and `out/b.json` exist.",
        "Call the writer twice and assert that `out/a.json` is present.",
        "Add a test that asserts `out/a.json` exists.",
        "After cleanup, assert `src/existing.rs` does not exist in the scratch copy.",
        "and assert: (a) `out/a.json` exists; (b) `out/b.md` exists; (c) the aggregate `out/c.md` exists;",
        "The test must assert:\n- `out/a.json` exists\n- (b) `out/b.md` exists and parses",
        // Hyphenated words and parentheses inside an item do not end the
        // enumeration.
        "and assert: (a) `out/a.json` exists and names run-real; (b) `out/b.md` exists (see the notes); (c) `out/c.md` exists;",
        "1. `case_one` — run it and assert:\n   - `out/a.json` exists\n   - `out/b.md` is absent",
    ] {
        assert!(
            claims(text).is_empty(),
            "{text:?} yielded {:?}",
            claims(text)
        );
    }
}

#[test]
fn issue443_repository_statements_beside_assertions_are_still_claims() {
    let ghost = |text: &str| {
        assert_eq!(
            claims(text),
            vec![("src/ghost.rs".to_string(), Claim::Exists)],
            "{text:?}"
        );
    };
    // No assertion verb at all.
    ghost("`src/ghost.rs` exists (241 lines).");
    ghost("Files Expected to Change: `src/ghost.rs` — exists");
    // `assert` as a noun, or inside code, is no lead.
    ghost("Add an assert to `src/ghost.rs`, which already exists.");
    ghost("Add `assert!(path.exists())` to `src/ghost.rs`, which already exists.");
    ghost("Each assertion lives in `src/ghost.rs`, which already exists.");
    // A word before the path is the body's own statement.
    ghost("Assert the parse result, then modify `src/ghost.rs`.");
    ghost("Assert that the loader returns `Ok` from the existing `src/ghost.rs`.");
    // A `;` without an enumeration marker, a clause word, a comma after a
    // plain word, or a long gap ends the assertion clause.
    ghost("The test asserts the totals; `src/ghost.rs` exists (12 lines).");
    ghost("Assert the totals then `src/ghost.rs` exists (12 lines).");
    ghost("Assert the totals match, `src/ghost.rs` exists (12 lines).");
    ghost(
        "Assert the report renders cleanly for every configured market and the review names `src/ghost.rs`, which already exists.",
    );
    // A checking verb without `that` or a test subject is no lead.
    ghost("Check `src/ghost.rs` exists.");
    // A sibling list item after an asserted list item is no longer asserted;
    // a plain paragraph line ends a list opened by `assert:`.
    ghost(
        "1. `case_one` — run it and assert:\n   - `out/a.json` exists\n2. `case_two` — `src/ghost.rs` exists (3 lines)",
    );
    ghost(
        "The test must assert:\n- `out/a.json` exists\n\nThe module `src/ghost.rs` already exists.",
    );
}

#[test]
fn issue443_true_repository_claims_are_still_refuted_beside_assertions() {
    let (_temp, project, _tasks, tree) = grounded();
    let text = "The test asserts that `out/report.json` exists. `src/ghost.rs` exists (12 lines). `src/existing.rs` does not exist yet.";
    let found = findings_against(&tree, &project, "TASK-X-443", text);
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(found[0].contains("`src/ghost.rs` exists"), "{found:?}");
    assert!(
        found[1].contains("`src/existing.rs` does not exist"),
        "{found:?}"
    );
    assert!(
        found.iter().all(|f| !f.contains("out/report.json")),
        "{found:?}"
    );
}

/// #366 (d) once skipped this sentence shape through a runtime-output phrase
/// list; d1d12efb6 removed that skip because output wording ("written by",
/// "produced by", "under the runtime data root") hid explicit repository
/// claims. The #443 rule keys on an assertion lead before the path instead,
/// so the asserted sentence is exempt and those claims still fire.
#[test]
fn issue443_restores_the_366d_assertion_without_the_d1d12efb6_false_negatives() {
    let asserted = "The appended `refreshes_frozen_review_aggregate` test lives in the `store_tests` module root (append-only): it runs `run_native_backtest` against a tempfile registry (reusing the shared `request()`/`backtest_config()` fixtures), asserts `backtests/<run-id>/adversarial-review.md` exists, asserts `backtests/adversarial-review.md` exists, and asserts the aggregate equals the latest run's review.";
    assert!(claims(asserted).is_empty(), "{:?}", claims(asserted));

    for (text, path) in [
        (
            "`backtests/adversarial-review.md` exists under the runtime data root and is generated by the runner.",
            "backtests/adversarial-review.md",
        ),
        (
            "`src/ghost.rs` exists (written by TASK-DL-001).",
            "src/ghost.rs",
        ),
        (
            "`src/ghost.rs` exists and is produced by the build script.",
            "src/ghost.rs",
        ),
        (
            "Modify `src/ghost.rs` so the report written by the runner carries the field.",
            "src/ghost.rs",
        ),
        (
            "The runner writes its output at run time; `src/ghost.rs` exists (4 lines).",
            "src/ghost.rs",
        ),
    ] {
        assert_eq!(
            claims(text),
            vec![(path.to_string(), Claim::Exists)],
            "{text:?}"
        );
    }
}

#[test]
fn evidence_366_asserted_output_and_repository_claims_remain_distinct() {
    let runtime_output =
        "The test asserts `out/report.json` exists under its temporary output root.";
    let beside_assertion =
        "The test asserts `out/report.json` exists. `src/ghost.rs` exists (12 lines).";
    let runtime_phrase_claim =
        "`src/ghost.rs` exists under the runtime data root and is produced by the runner.";
    let cases = [
        (
            runtime_output,
            vec!["out/report.json".to_string()],
            Vec::<String>::new(),
        ),
        (
            beside_assertion,
            vec!["out/report.json".to_string(), "src/ghost.rs".to_string()],
            vec!["src/ghost.rs".to_string()],
        ),
        (
            runtime_phrase_claim,
            Vec::<String>::new(),
            vec!["src/ghost.rs".to_string()],
        ),
    ];
    for (case, before, expected) in cases {
        let actual = claimed_paths(case);
        println!("EVIDENCE #366 assertion-claim BEFORE: {before:?} AFTER: {actual:?}");
        assert_eq!(actual, expected, "{case}");
    }
}

fn claimed_paths(text: &str) -> Vec<String> {
    claims(text).into_iter().map(|(path, _)| path).collect()
}

#[test]
fn issue461_reported_and_call_syntax_paths_are_not_repository_claims() {
    for text in [
        "The claim that `src/ghost.rs` exists is rewritten to what is observed.",
        "The statement that `src/ghost.rs` exists was repeated.",
        "The assumption that `src/ghost.rs` exists is unsupported.",
        "The assertion that `src/ghost.rs` exists was reported.",
        "The reviewer says that `src/ghost.rs` exists.",
        "The reviewer said that `src/ghost.rs` exists.",
        "The reviewer states that `src/ghost.rs` exists.",
        "The reviewer stated that `src/ghost.rs` exists.",
        "The reviewer asserts that `src/ghost.rs` exists.",
        "The reviewer claims that `src/ghost.rs` exists.",
        "The reviewer assumes that `src/ghost.rs` exists.",
        "The claim that `src/ghost.rs` and `src/other-ghost.rs` exist was reported.",
        "The body says `src/ghost.rs` exists.",
        "`ahdm_strategy_root()/strategy-spec.json` exists.",
    ] {
        assert!(claims(text).is_empty(), "{text:?}: {:?}", claims(text));
    }
}

#[test]
fn issue461_ambiguous_negation_and_report_leads_are_scoped_to_their_lists() {
    let (_temp, project, _tasks, tree) = grounded();
    for text in [
        "No changes are needed here; `src/ghost.rs` exists.",
        "There is no migration, and `src/ghost.rs` exists.",
    ] {
        let found = findings_against(&tree, &project, "TASK-X-461", text);
        assert_eq!(found.len(), 1, "{text:?}: {found:?}");
        assert!(found[0].contains("`src/ghost.rs` exists"), "{found:?}");
    }

    let reported_list = "The reviewer claims that `src/ghost.rs` and `src/ghost2.rs` exist.";
    assert!(
        claims(reported_list).is_empty(),
        "{:?}",
        claims(reported_list)
    );

    let unreported_list = "This task claims AC-1; `src/ghost.rs` and `src/ghost2.rs` exist.";
    assert_eq!(
        claims(unreported_list),
        vec![("src/ghost2.rs".to_string(), Claim::Exists)]
    );

    let essays = "The essays say `src/ghost.rs` exists.";
    assert_eq!(
        claims(essays),
        vec![("src/ghost.rs".to_string(), Claim::Exists)]
    );
}

#[test]
fn issue461_repository_claim_changes_never_widen_coordinated_paths() {
    // The extraction change must be a subset of the base claims for existing
    // repository assertions; only an unambiguous negated subject adds Absent.
    let cases = [
        (
            "`src/a.rs` and `src/b.rs` exist.",
            vec![("src/b.rs".to_string(), Claim::Exists)],
        ),
        (
            "Either `src/a.rs` or `src/b.rs` exists.",
            vec![("src/b.rs".to_string(), Claim::Exists)],
        ),
        (
            "The reviewer claims that `src/a.rs`, `src/b.rs`, and `src/c.rs` exist.",
            vec![],
        ),
        (
            "No path of the form `src/a.rs` exists.",
            vec![("src/a.rs".to_string(), Claim::Absent)],
        ),
        (
            "No changes are needed here; `src/b.rs` exists.",
            vec![("src/b.rs".to_string(), Claim::Exists)],
        ),
    ];

    for (sentence, expected) in cases {
        assert_eq!(claims(sentence), expected, "{sentence:?}");
    }
}

#[test]
fn issue461_negated_noun_phrases_are_absence_claims_and_siblings_still_check() {
    for text in [
        "No path of the form `src/ghost.rs` exists.",
        "No such file `src/ghost.rs` exists.",
        "No file named `src/ghost.rs` exists.",
        "No other module `src/ghost.rs` exists.",
        "There is no file `src/ghost.rs` present.",
        "None of the files `src/ghost.rs` exist.",
    ] {
        assert_eq!(
            claims(text),
            vec![("src/ghost.rs".to_string(), Claim::Absent)],
            "{text:?}"
        );
    }

    for text in [
        "No evidence about the planned file `src/ghost.rs` exists.",
        "No test that reads the file `src/ghost.rs` exists.",
        "No existing file `src/ghost.rs` exists.",
    ] {
        assert!(claims(text).is_empty(), "{text:?}");
    }

    let (_temp, project, _tasks, tree) = grounded();
    let text = "No file named `src/existing.rs` exists. `src/ghost.rs` exists.";
    let found = findings_against(&tree, &project, "TASK-X-461", text);
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(
        found[0].contains("`src/existing.rs` does not exist"),
        "{found:?}"
    );
    assert!(found[1].contains("`src/ghost.rs` exists"), "{found:?}");
}

#[test]
fn issue461_live_rewrites_and_ambiguous_negations_make_no_false_finding() {
    let (_temp, project, _tasks, tree) = grounded();
    for text in [
        "No path of the form `ahdm_strategy_root()/strategy-spec.json` exists under the repository root;",
        "BLOCKING repository claims: the body says `ahdm_strategy_root()/strategy-spec.json` exists (...) but it is absent from repository ...",
        "(b) The claim that `ahdm_strategy_root()/strategy-spec.json` exists at base commit is rewritten to what is observed: ...",
        "No convincing evidence `src/ghost.rs` exists.",
    ] {
        assert!(
            findings_against(&tree, &project, "TASK-X-461", text).is_empty(),
            "{text:?}"
        );
    }
    // Existing true repository claims remain eligible beside the negated sentence.
    let mixed = "No path of the form `src/ghost.rs` exists. `src/other-ghost.rs` exists.";
    let found = findings_against(&tree, &project, "TASK-X-461", mixed);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found[0].contains("`src/other-ghost.rs` exists"),
        "{found:?}"
    );
}

#[path = "repository_claims_review_tests.rs"]
mod review_tests;
