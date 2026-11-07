//! Host actions are screened; states produced by the work remain findings.
use super::super::continuation_tests::{Scripted, contract};
use super::tests::reply;
use super::*;
use archon_workflow::task_set_contract::{AcceptanceCheck, TrustedCwd};

fn fixture() -> (AcceptanceContract, JudgeRoots) {
    let mut subject = contract();
    subject.acceptance[0].check = AcceptanceCheck::Command {
        command: "# guard.rs\nrun-tests".into(),
        cwd: TrustedCwd::ProjectRoot,
    };
    (subject, JudgeRoots::default())
}

const MUTATIONS: &[&str] = &[
    "The host moves guard.rs aside, cargo test passes trivially.",
    "The host moves guard.rs aside, so cargo test passes trivially with no matching tests.",
    "The host renames guard.rs; no tests run.",
    "The host renames guard.rs. The check passes.",
    "The host renames trading_data_tests.rs. No tests run.",
    "The host renames guard.test.rs to saved.test.rs. The check passes.",
    "The host renames `other.rs` to `saved.rs`. The check passes.",
    "The host puts guard.rs aside. The check passes.",
    "The host’s probe moves guard.rs aside. No assertions remain.",
    "The host's mutation probe moves guard.rs aside. The check passes.",
    "The host mutation probe renames guard.rs. The check passes.",
    "The mutation probe puts guard.rs aside. The check passes.",
];

#[test]
fn host_actions_reask_then_forward_persistent_refutations() {
    for state in MUTATIONS {
        let (mut subject, roots) = fixture();
        let content = reply("state", state, "The check passes.", &[]);
        let mut screen = Screen::default();
        for attempt in 0..3 {
            let missing = screen.apply(&mut subject, &content, &roots).unwrap();
            assert_eq!(missing.is_empty(), attempt == 2, "{state}");
            if attempt < 2 {
                assert_eq!(missing, BTreeSet::from(["AC-X-001".into()]));
                assert!(
                    screen
                        .instruction(&subject)
                        .contains(mutation::MUTATION_FACT)
                );
            }
        }
        assert_eq!(
            subject.acceptance[0].judgment.verdict,
            JudgeDecision::Refuted
        );
        assert_eq!(subject.acceptance[0].judgment.counterexample, *state);
        assert!(
            subject.acceptance[0]
                .judgment
                .reason
                .starts_with("[kind=state]")
        );
        assert!(screen.instruction(&subject).is_empty());
    }
}

#[test]
fn implementation_states_and_unrelated_actions_survive() {
    for state in [
        "The implementation removes guard.rs. The check passes.",
        "The work moves guard.rs aside. The check passes.",
        "A stale copy removes guard.rs while the check runs the old assertion.",
        "The state itself changes guard.rs. The check passes.",
        "guard.rs is removed. The check passes.",
        "data.csv is empty. The check passes.",
        "The host deletes guard.rs. The check passes.",
        "The host empties guard.rs. The check passes.",
        "The host does not move guard.rs aside. The implementation skips validation.",
        "The implementation leaves validation aside and returns success.",
        "The host moves guard.rs to another directory. An unrelated aside follows.",
        "The host moves the test file aside. The check passes.",
        "The host renames the test file. The check passes.",
        "The implementation returns wrong values but the assertion checks only length.",
        "The host renames guard.rs so the implementation's validation is skipped",
        "The host renames trading_data_tests.rs's test functions, so --exact matches zero tests",
        "The host renames guard.test.rs so the check passes.",
        "The host moves guard.rs aside. Separately, the implementation maps the validation error to Ok",
        "The host moves guard.rs aside, then the implementation renames the deciding tests so the check passes.",
        "The host moves guard.rs aside. The work skips validation.",
        "The host moves guard.rs aside. Validation errors then become Ok and the check passes.",
        "The host renames guard.rs to guard.bak and validation errors become Ok.",
        "The deciding tests do not exist yet, so zero tests run.",
        "The test functions were never written, so --exact matches zero tests.",
        "Validation errors become warnings and the run succeeds.",
        "The validator accepts tampered metadata.",
        "The host moves guard.rs aside. The deciding tests do not exist yet, so zero tests run.",
        "The host moves guard.rs aside. The test functions were never written, so --exact matches zero tests.",
        "The host moves guard.rs aside. Validation errors become warnings and the run succeeds.",
        "The host moves guard.rs aside. The validator accepts tampered metadata.",
        "The host moves guard.rs aside. Separately, the assertion checks only length.",
        "The host moves guard.rs aside, then the assertion compares only length.",
        "The host moves nothing aside; the check passes.",
        "The host moves no objects aside; the check passes.",
        "The host puts none aside; the check passes.",
        "The host renames nothing; the check passes.",
        "The host renames nothing to guard.rs; the check passes.",
        "The host does not move guard.rs aside; the check passes.",
        "The host moves guard.rs aside; an unfamiliar outcome follows.",
        "The host moves guard.rs aside; the error is mapped to Ok and ignored.",
        "The host moves guard.rs aside; validation was bypassed and the value hardcoded.",
        "The host renames guard.rs to guard.bak so validation is skipped.",
        "The host renames guard.rs to guard.bak; the error is mapped to Ok and ignored.",
        "The host renames guard.rs to guard.bak, validation was bypassed and the value hardcoded.",
        "The host renames guard.rs to guard.bak. The deciding tests are deleted, so --exact matches zero tests.",
        "The host moves guard.rs aside; the deciding tests are renamed.",
        "The host moves guard.rs aside; the deciding tests are absent.",
        "The host moves guard.rs aside; the deciding test was removed by the refactor.",
        "The host moves guard.rs aside. The renamed tests match nothing.",
        "The host moves guard.rs aside; the validate command exits zero on bad data.",
        // Unrecognised consequences remain findings.
        "The host's mutation probe moves guard.rs aside. The command exits zero.",
        "The host mutation probe renames guard.rs. The command exits zero.",
        "The mutation probe puts guard.rs aside. The command exits zero.",
    ] {
        let (mut subject, roots) = fixture();
        let mut screen = Screen::default();
        assert!(
            screen
                .apply(
                    &mut subject,
                    &reply("state", state, "The check passes.", &[]),
                    &roots
                )
                .unwrap()
                .is_empty(),
            "{state}"
        );
        assert_eq!(subject.acceptance[0].judgment.counterexample, state);
        assert_eq!(subject.acceptance[0].judgment.reason, "The check passes.");
        assert!(screen.instruction(&subject).is_empty());
    }
}

const INDEPENDENT_REASONS: &[&str] = &[
    "The implementation maps the validation error to Ok.",
    "The work omits validation, and the check passes.",
    "Separately, the assertion checks only length.",
    "The assertion compares only length, allowing incorrect values.",
    "Validation is disabled, and the check still passes.",
    "An empty collection passes vacuously.",
    "The error is mapped to Ok and ignored.",
    "Validation was bypassed and the value hardcoded.",
    "The deciding tests are deleted, so --exact matches zero tests.",
    "The deciding tests are renamed.",
    "The deciding tests are absent.",
    "The deciding test was removed by the refactor.",
    "The renamed tests match nothing.",
    "The validate command exits zero on bad data.",
    "Validation errors then become Ok and the check passes.",
    "The deciding tests do not exist yet, so zero tests run.",
    "The test functions were never written, so --exact matches zero tests.",
    "Validation errors become warnings and the run succeeds.",
    "The validator accepts tampered metadata.",
    "An unfamiliar outcome follows.",
];

#[test]
fn host_action_with_independent_reason_survives() {
    for reason in INDEPENDENT_REASONS {
        let (mut subject, roots) = fixture();
        let mut screen = Screen::default();
        let state = MUTATIONS[0];
        assert!(
            screen
                .apply(&mut subject, &reply("state", state, reason, &[]), &roots)
                .unwrap()
                .is_empty(),
            "{reason}"
        );
        assert_eq!(subject.acceptance[0].judgment.counterexample, state);
        assert_eq!(subject.acceptance[0].judgment.reason, *reason);
        assert!(screen.instruction(&subject).is_empty());
    }
}

#[test]
fn accepted_host_actions_are_never_screened() {
    for state in MUTATIONS {
        let (mut subject, roots) = fixture();
        let content =
            reply("other", state, "The check passes.", &[]).replace("\"refuted\"", "\"accepted\"");
        let mut screen = Screen::default();
        assert!(
            screen
                .apply(&mut subject, &content, &roots)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            subject.acceptance[0].judgment.verdict,
            JudgeDecision::Accepted
        );
        assert!(screen.instruction(&subject).is_empty());
    }
}

#[tokio::test]
async fn mutation_reask_keeps_other_entries_and_accepts_a_corrected_reply() {
    let (mut subject, roots) = fixture();
    let mut second = subject.acceptance[0].clone();
    second.id = "AC-X-002".into();
    subject.acceptance.push(second);
    let mut first: serde_json::Value =
        serde_json::from_str(&reply("state", MUTATIONS[0], "The check passes.", &[])).unwrap();
    first["decisions"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "id":"AC-X-002", "verdict":"accepted", "kind":"other", "cited_paths":[],
            "counterexample":"None found.", "reason":"The assertion decides the check."
        }));
    let first = first.to_string();
    let corrected = reply(
        "other",
        "None found.",
        "The assertion decides the check.",
        &[],
    )
    .replace("\"refuted\"", "\"accepted\"");
    let client = Scripted::new(vec![
        Ok((&first, Some("end_turn"))),
        Ok((&corrected, Some("end_turn"))),
    ]);
    let judged = judge_batch(
        &client,
        subject,
        "model",
        require_judged_prose,
        None,
        &roots,
    )
    .await
    .unwrap();
    assert!(
        judged
            .acceptance
            .iter()
            .all(|entry| entry.judgment.verdict == JudgeDecision::Accepted)
    );
    let calls = client.calls();
    assert_eq!(calls.len(), 2);
    let ask = calls[1][0]["content"].as_str().unwrap();
    assert!(ask.contains(mutation::MUTATION_FACT));
    assert!(ask.contains("AC-X-001"));
    assert!(!ask.contains("AC-X-002"));
}

#[test]
fn recorded_host_action_is_reasked() {
    let (mut subject, roots) = fixture();
    let mut entry = subject.acceptance.remove(0);
    entry.id = "SUP-REQ-DL-123".into();
    entry.criterion = "Commands must return non-zero on failed validation unless an explicit diagnostic flag is supplied.".into();
    entry.covers = vec!["REQ-DL-123".into()];
    entry.gap_permitted = false;
    entry.check = AcceptanceCheck::Command {
        command: r#"set -eu
# SUP-REQ-DL-123 -- REQ-DL-123: commands must return non-zero on failed
# validation unless an explicit diagnostic flag is supplied.
#
# Deciding assertions (their exit status decides this check; a failure in any
# one fails it). The literal test-file paths below are read only so the host
# can move them aside to prove this check can fail -- the deciding assertion is
# always the cargo test run's exit status:
#
# 1. src/command/trading_data_tests.rs
#    trading_data_validate_rejects_tampered_metadata_before_revalidation
#    Drives the real `render_data` dispatch arm for
#    TradingCliDataAction::Validate and asserts `result.is_err()` on a failed
#    validation: the command surface propagates the failure (rendered non-zero
#    by main's Result-bearing exit) instead of converting it to an Ok report.
#    If validate() ever maps the failure to Ok via data_error/write_or_render,
#    this test fails and so does this check -- exactly the regression
#    REQ-DL-123 forbids (exit 0 on failed validation without a diagnostic flag).
# 2. src/command/trading_data_tests.rs
#    trading_data_validate_dispatches_with_target
#    Pins the success lane: a passing validation renders the report through
#    the same arm, so a check that only ever asserted failure could not tell a
#    broken dispatch from a broken exit contract; this keeps the failure test
#    meaningful by proving the arm still runs when validation passes.
#
# Both tests live in the bin target's `command::trading_data::tests` module of
# package `archon-cli-workspace` (the package named by the workspace manifest);
# `--exact` with the full module-qualified paths matches them at base.
cargo test -p archon-cli-workspace --bin archon \
  command::trading_data::tests::trading_data_validate_rejects_tampered_metadata_before_revalidation \
  command::trading_data::tests::trading_data_validate_dispatches_with_target \
  -- --exact"#.into(),
        cwd: TrustedCwd::RepoRoot,
    };
    subject.supplementary.push(entry);
    let counterexample = "The host moves src/command/trading_data_tests.rs aside, cargo test --exact passes trivially with no matching tests, matching zero tests is success for cargo test so the check passes while REQ-DL-123 has no deciding test.";
    // Verbatim counterexample and reason from the recorded freeze-cache reply.
    let reason = "cargo test with --exact and zero matches exits 0, so moving the test file aside yields a passing check over a false state with no behavioural proof.";
    for kind in ["state", "coverage"] {
        let mut subject = subject.clone();
        let content =
            reply(kind, counterexample, reason, &[]).replace("AC-X-001", "SUP-REQ-DL-123");
        let mut screen = Screen::default();
        for attempt in 0..3 {
            let missing = screen.apply(&mut subject, &content, &roots).unwrap();
            if attempt < 2 {
                assert_eq!(missing, BTreeSet::from(["SUP-REQ-DL-123".into()]));
                assert!(
                    screen
                        .instruction(&subject)
                        .contains(mutation::MUTATION_FACT)
                );
            } else {
                assert!(missing.is_empty());
                assert_eq!(
                    subject.supplementary[0].judgment.verdict,
                    JudgeDecision::Refuted
                );
                assert_eq!(
                    subject.supplementary[0].judgment.counterexample,
                    counterexample
                );
                assert!(
                    subject.supplementary[0]
                        .judgment
                        .reason
                        .starts_with(&format!("[kind={kind}]"))
                );
            }
        }
    }
}

#[test]
fn coverage_supplies_cited_content_before_mutation_reasks() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("cited.rs"), "assert!(actual > 0);\n").unwrap();
    for already_supplied in [false, true] {
        let (mut subject, _) = fixture();
        if already_supplied {
            subject.acceptance[0].check = AcceptanceCheck::Command {
                command: "cat cited.rs".into(),
                cwd: TrustedCwd::ProjectRoot,
            };
        }
        let roots = JudgeRoots {
            project: Some(root.path().into()),
            task_root: Some(root.path().into()),
            ..Default::default()
        };
        let content = reply("coverage", MUTATIONS[0], "The check passes.", &["cited.rs"]);
        let mut screen = Screen::default();
        for attempt in usize::from(already_supplied)..4 {
            let missing = screen.apply(&mut subject, &content, &roots).unwrap();
            let instruction = screen.instruction(&subject);
            if attempt == 0 {
                assert_eq!(missing, BTreeSet::from(["AC-X-001".into()]));
                assert!(instruction.contains("cited named_files supplied"));
                assert!(instruction.contains("assert!(actual > 0)"));
                assert!(!instruction.contains(mutation::MUTATION_FACT));
            } else if attempt < 3 {
                assert_eq!(missing, BTreeSet::from(["AC-X-001".into()]));
                assert!(instruction.contains(mutation::MUTATION_FACT));
            } else {
                assert!(missing.is_empty());
                assert!(instruction.is_empty());
                assert_eq!(subject.acceptance[0].judgment.counterexample, MUTATIONS[0]);
                assert!(
                    subject.acceptance[0]
                        .judgment
                        .reason
                        .starts_with("[kind=coverage]")
                );
            }
        }
    }
}
