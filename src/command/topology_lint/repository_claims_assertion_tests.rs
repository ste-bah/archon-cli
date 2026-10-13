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

/// The hostile review of #443: every case with the paths it must claim.
/// Each case opens with its review label. Kept exempt on purpose: an
/// imperative or test-subject check with no repository wording (FN3-FN6,
/// FN18, FN21, G9, G12: "verify that `x` exists" is what to check, not a
/// statement that `x` is there), a continued complement (FN15, K3: "and that
/// `y` exists"), and G8, which no build claims ("exists to produce" ends no
/// predicate).
#[test]
fn issue443_review_cases_claim_exactly_the_repository_statements() {
    let cases: &[(&str, &[&str])] = &[
        (
            r#"SANITY-TN run the producer, and assert: (a) `out/run/report.json` exists; (b) `out/run/review.md` exists and contains x; (c) the aggregate `out/review.md` exists;"#,
            &[],
        ),
        (
            r#"SANITY-TP `src/ghost.rs` exists (12 lines)."#,
            &["src/ghost.rs"],
        ),
        (
            r#"FN1 The test asserts the parser in `src/ghost.rs` exists and handles empty input."#,
            &["src/ghost.rs"],
        ),
        (
            r#"FN2 Before editing, verify that `crates/y/src/ghost.rs` exists (240 lines)."#,
            &["crates/y/src/ghost.rs"],
        ),
        (
            r#"FN3 Files Expected to Change — assert: `src/ghost.rs` exists."#,
            &[],
        ),
        (
            r#"FN4 Confirm that `src/ghost.rs` already exists; it defines `Parser`."#,
            &[],
        ),
        (
            r#"FN5 Ensure that `src/ghost.rs` exists before you modify it."#,
            &[],
        ),
        (
            r#"FN6 Check that `src/ghost.rs` exists, then extend it."#,
            &[],
        ),
        (
            r#"FN7-indented-opener
1. `test_a` — run the producer.
   The test must assert:
   - `out/a.json` exists
2. `src/ghost.rs` exists (12 lines) and holds the parser.
3. `src/ghost2.rs` already exists and is modified."#,
            &["src/ghost.rs", "src/ghost2.rs"],
        ),
        (
            r#"FN8-blank-line-swallow
The test must assert:
- `out/a.json` exists

- `src/ghost.rs` exists (12 lines)
- `src/ghost2.rs` is present"#,
            &["src/ghost.rs", "src/ghost2.rs"],
        ),
        (
            r#"FN9-marker-reopen Keep the unit test that asserts the rounding, and note: (a) `src/ghost.rs` exists (12 lines)."#,
            &["src/ghost.rs"],
        ),
        (
            r#"FN10-semicolon-carry The new test asserts the rounding; (b) `src/ghost.rs` exists (12 lines)."#,
            &["src/ghost.rs"],
        ),
        (
            r#"FN11 We assert `src/ghost.rs` already exists in the repository."#,
            &["src/ghost.rs"],
        ),
        (
            r#"FN12-list-of-context
Each test should verify that:
- the output is non-empty
- `src/ghost.rs` exists at the base commit"#,
            &["src/ghost.rs"],
        ),
        (
            r#"FN13-heading-not-list
The tests assert:
- `out/a.json` exists
1) `src/ghost.rs` exists"#,
            &["src/ghost.rs"],
        ),
        (
            r#"FN14 Assert `out/a.json` exists and `src/ghost.rs` exists (12 lines, at the base commit)."#,
            &["src/ghost.rs"],
        ),
        (
            r#"FN15-words Assert `out/a.json` exists and that `src/ghost.rs` exists."#,
            &[],
        ),
        (
            r#"FN16 The test verifies that the loader in `src/ghost.rs` is present."#,
            &["src/ghost.rs"],
        ),
        (
            r#"FN17 assert: x; Note `src/ghost.rs` exists."#,
            &["src/ghost.rs"],
        ),
        (r#"FN18 tests check `src/ghost.rs` exists"#, &[]),
        (
            r#"FN19-numbered-list-opener-plain
Context:
The existing tests assert:
1. `src/ghost.rs` exists (12 lines)
2. `src/lib.rs` will be modified"#,
            &["src/ghost.rs", "src/lib.rs"],
        ),
        (
            r#"FN20-bold-list
**The test must assert:**
- `out/a.json` exists"#,
            &[],
        ),
        (
            r#"FN21 Verify that the file `crates/x/src/ghost.rs` exists and add a function.
"#,
            &[],
        ),
        (
            r#"G1-indented-opener-dash-siblings
- `test_a` — run the producer.
  The test must assert:
  - `out/a.json` exists
- `src/ghost.rs` exists (12 lines) and holds the parser.
- `src/ghost2.rs` already exists."#,
            &["src/ghost.rs", "src/ghost2.rs"],
        ),
        (
            r#"G2-numbered-children-under-assert
The test must assert:
1. `out/a.json` exists
2. `out/b.json` exists"#,
            &[],
        ),
        (
            r#"G3-paren-children
The test must assert:
1) `out/a.json` exists
2) `src/ghost.rs` exists"#,
            &[],
        ),
        (
            r#"G4-inline-numbered And assert: 1. `out/a.json` exists; 2. `out/b.json` exists."#,
            &[],
        ),
        (
            r#"G5 The test asserts `out/a.json` exists; (b) `src/ghost.rs` exists (12 lines, already in the repository)."#,
            &["src/ghost.rs"],
        ),
        (
            r#"G6-files-section-after-assert-list
The test must assert:
- `out/a.json` exists
- `src/ghost.rs` — exists (206 lines)"#,
            &["src/ghost.rs"],
        ),
        (
            r#"G7-four-words The test asserts the files under `src/ghost.rs` exist."#,
            &["src/ghost.rs"],
        ),
        (
            r#"G8 Assert the output, which `src/ghost.rs` already exists to produce."#,
            &[],
        ),
        (
            r#"G9-confirm-in-step 2. Confirm that `crates/x/src/ghost.rs` exists and exports `Parser`; then extend it."#,
            &[],
        ),
        (
            r#"G10 Expect that `src/ghost.rs` is present in the checkout."#,
            &["src/ghost.rs"],
        ),
        (
            r#"G11-asserts-noun-before The asserts in `src/ghost.rs` already exist."#,
            &["src/ghost.rs"],
        ),
        (r#"G12 Tests verify `src/ghost.rs` exists."#, &[]),
        (
            r#"G13-carry-sentence2
The test must assert:
- `out/a.json` exists. `src/ghost.rs` exists (12 lines)."#,
            &["src/ghost.rs"],
        ),
        (
            r#"G14-carry-second-item-sentence
The test must assert:
- `out/a.json` exists; (b) `src/ghost.rs` exists (12 lines)."#,
            &["src/ghost.rs"],
        ),
        (
            r#"G15 Assert `out/a.json` exists — the parser `src/ghost.rs` exists (12 lines)."#,
            &["src/ghost.rs"],
        ),
        (
            r#"G16 Assert `out/a.json` exists and the parser `src/ghost.rs` exists (12 lines)."#,
            &["src/ghost.rs"],
        ),
        (
            r#"G17 Assert that `out/a.json` exists and `src/ghost.rs` is new.
"#,
            &["src/ghost.rs"],
        ),
        (
            r#"R1
## Tests
- `report_is_written` — run the producer on a temp lake.
  The test must assert:
  - `out/run/report.json` exists
- `parser_is_reused` — extend the parser; `crates/x/src/ghost.rs` already exists (240 lines).
- `crates/x/src/ghost2.rs` exists (12 lines) and holds the loader."#,
            &["crates/x/src/ghost.rs", "crates/x/src/ghost2.rs"],
        ),
        (
            r#"R2-control-without-indented-opener
## Tests
- `report_is_written` — run the producer on a temp lake.
- `crates/x/src/ghost2.rs` exists (12 lines) and holds the loader.
"#,
            &["crates/x/src/ghost2.rs"],
        ),
        (
            r#"P1 ## Tests
- `report_is_written` — run the producer on a temp lake.
  The test must assert:
  - `out/run/report.json` exists
- `crates/x/src/ghost2.rs` exists and holds the loader."#,
            &["crates/x/src/ghost2.rs"],
        ),
        (
            r#"P2a The test must assert:
1. `out/a.json` exists
2. `out/b.json` exists"#,
            &[],
        ),
        (
            r#"P2b And assert: 1. `out/a.json` exists; 2. `out/b.json` exists."#,
            &[],
        ),
        (
            r#"P3 Assert `out/a.json` exists and the parser `src/ghost.rs` exists."#,
            &["src/ghost.rs"],
        ),
        (
            r#"P4a Keep the unit test that asserts the rounding, and note: (a) `src/ghost.rs` exists."#,
            &["src/ghost.rs"],
        ),
        (
            r#"P4b Assert: (a) `out/a.json` exists, and note: (b) `src/ghost.rs` exists."#,
            &["src/ghost.rs"],
        ),
        (
            r#"P4c The new test asserts the rounding; (b) `src/ghost.rs` exists."#,
            &["src/ghost.rs"],
        ),
        (r#"K1 Assert `out/a.json` and `out/b.json` exist."#, &[]),
        (
            r#"K2 The test must assert:
- `out/a.json` exists
- `out/b.json` exists
- `src/ghost.rs` will be modified"#,
            &["src/ghost.rs"],
        ),
        (
            r#"K3 Assert `out/a.json` exists and that `out/b.json` exists.
"#,
            &[],
        ),
    ];
    for (text, expected) in cases {
        assert_eq!(claimed_paths(text), *expected, "{text:?}");
    }
}

/// The four review findings, each without line counts or repository
/// wording, so the rule itself is what decides.
#[test]
fn issue443_review_findings_hold_without_repository_wording() {
    // 1. An indented paragraph opener carries only the items nested under it.
    assert_eq!(
        claimed_paths(
            "## Tests\n- `report_is_written` — run the producer on a temp lake.\n  The test must assert:\n  - `out/run/report.json` exists\n- `crates/x/src/ghost2.rs` exists and holds the loader."
        ),
        ["crates/x/src/ghost2.rs"]
    );
    // 2. Numbered items under an assertion are its items.
    assert!(
        claimed_paths("The test must assert:\n1. `out/a.json` exists\n2. `out/b.json` exists")
            .is_empty()
    );
    assert!(
        claimed_paths("And assert: 1. `out/a.json` exists; 2. `out/b.json` exists.").is_empty()
    );
    // 3. The asserted clause ends at the first asserted path's predicate.
    assert_eq!(
        claimed_paths("Assert `out/a.json` exists and the parser `src/ghost.rs` exists."),
        ["src/ghost.rs"]
    );
    // 4. A marker continues only an enumeration its lead opened, unbroken.
    for text in [
        "Keep the unit test that asserts the rounding, and note: (a) `src/ghost.rs` exists.",
        "Assert: (a) `out/a.json` exists, and note: (b) `src/ghost.rs` exists.",
        "The new test asserts the rounding; (b) `src/ghost.rs` exists.",
    ] {
        assert_eq!(claimed_paths(text), ["src/ghost.rs"], "{text:?}");
    }
}
