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

fn claimed_paths(text: &str) -> Vec<String> {
    claims(text).into_iter().map(|(path, _)| path).collect()
}

#[path = "repository_claims_review_tests.rs"]
mod review_tests;
