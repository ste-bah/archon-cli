//! #443 hostile-review cases for the asserted-clause rule.

use super::*;

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

/// Re-review of #443: shapes the tightened rule must still leave exempt.
#[test]
fn issue443_rereview_runtime_shapes_stay_exempt() {
    for text in [
        // A: a blank line between the opener and its first item.
        "The test must assert:\n\n- `out/a.json` exists\n- `out/b.md` exists",
        "**Each test asserts:**\n\n1. `out/a.json` exists",
        // C: a preposition that locates a run-time output.
        "Run the producer and assert the report at `out/run/report.json` exists.",
        "Assert the aggregate under `out/review.md` exists and parses.",
        // D: a prefixed checking verb.
        "At execution time re-verify that `providers/mod.rs` exists and its trait signature matches.",
    ] {
        assert!(
            claims(text).is_empty(),
            "{text:?} yielded {:?}",
            claims(text)
        );
    }
    // A blank line after the first item still ends the list; repository
    // wording still claims under an output noun.
    assert_eq!(
        claimed_paths("The test must assert:\n- `out/a.json` exists\n\n- `src/ghost.rs` exists"),
        ["src/ghost.rs"]
    );
    assert_eq!(
        claimed_paths("Assert the report at `src/ghost.rs` exists (12 lines)."),
        ["src/ghost.rs"]
    );
    assert_eq!(
        claimed_paths("Assert the report at `src/ghost.rs` exists in the repository."),
        ["src/ghost.rs"]
    );
}
