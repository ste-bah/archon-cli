//! The judge's reply is model output, and is unwrapped like any other.

use super::*;
use archon_workflow::task_set_contract::{
    AcceptanceCheck, AcceptanceContract, AcceptanceCriterion, GapPolicy, JudgeDecision,
    JudgeVerdict, PrdIdentity, TrustedCwd,
};

fn command_check(command: &str, cwd: TrustedCwd) -> AcceptanceContract {
    let mut subject = contract();
    subject.acceptance[0].check = AcceptanceCheck::Command {
        command: command.into(),
        cwd,
    };
    subject
}

fn git(root: &std::path::Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {:?}: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}

fn prompt_json(prompt: &str) -> serde_json::Value {
    serde_json::from_str(prompt.split_once("Checks: ").unwrap().1).unwrap()
}

fn contract() -> AcceptanceContract {
    AcceptanceContract {
        schema_version: 1,
        prd: PrdIdentity {
            path: "p".into(),
            digest: "d".into(),
        },
        gap_policy: GapPolicy {
            permitted_acceptance_ids: Default::default(),
            forbidden_phrases: Vec::new(),
            required_fields: Vec::new(),
        },
        acceptance: vec![AcceptanceCriterion {
            id: "AC-X-001".into(),
            criterion: "c".into(),
            check: AcceptanceCheck::Command {
                command: "true".into(),
                cwd: TrustedCwd::ProjectRoot,
            },
            gap_permitted: false,
            covers: Vec::new(),
            judgment: JudgeVerdict {
                verdict: JudgeDecision::Refuted,
                counterexample: String::new(),
                reason: String::new(),
                host_call_id: String::new(),
                sampling: None,
            },
        }],
        supplementary: Vec::new(),
    }
}

const DECISIONS: &str = r#"{"decisions":[{"id":"AC-X-001","verdict":"accepted","counterexample":"none","reason":"ok"}]}"#;

#[test]
fn a_fenced_judge_reply_is_accepted() {
    let mut subject = contract();

    apply_judgments(
        &mut subject,
        &format!("Here:\n\n```json\n{DECISIONS}\n```\n"),
    )
    .expect("a fenced judge reply is still an answer");

    assert_eq!(
        subject.acceptance[0].judgment.verdict,
        JudgeDecision::Accepted
    );
}

#[test]
fn a_judge_reply_followed_by_commentary_is_accepted() {
    let mut subject = contract();

    apply_judgments(
        &mut subject,
        &format!("{DECISIONS}\n\nThat satisfies the PRD.\n"),
    )
    .expect("commentary after the document does not hide it");

    assert_eq!(
        subject.acceptance[0].judgment.verdict,
        JudgeDecision::Accepted
    );
}

#[test]
fn a_reply_with_no_document_is_still_refused() {
    let error = apply_judgments(&mut contract(), "I could not decide.")
        .expect_err("prose carrying no decisions is not an answer");

    assert!(
        format!("{error:#}").contains("malformed batched JSON"),
        "{error:#}"
    );
}

/// A judge allowed to imagine any filesystem state can stub the program under
/// test and refute every command check forever. The prompt must fix the
/// toolchain and let only implementation-produced states vary.
#[test]
fn the_judge_prompt_fixes_the_toolchain_and_bounds_counterexamples() {
    let prompt = batched_judge_prompt(&contract()).expect("prompt");
    for phrase in [
        "every executable that the repository does not itself build are out of bounds",
        "must not refute a check",
        "the repository's own source and the program it builds from that source",
        "in-bounds state where the check passes while the criterion is false",
        "fallible, not adversarial",
        "under it no check could ever pass",
        "\"verdict\":\"accepted|refuted\"",
    ] {
        assert!(prompt.contains(phrase), "missing: {phrase}");
    }
    assert!(prompt.contains("AC-X-001"));
}

#[path = "workflow_task_set_judge_tests_named_files.rs"]
mod named_files_tests;
