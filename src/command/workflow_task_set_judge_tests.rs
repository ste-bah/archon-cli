//! The judge's reply is model output, and is unwrapped like any other.

use super::*;
use archon_workflow::task_set_contract::{
    AcceptanceCheck, AcceptanceContract, AcceptanceCriterion, GapPolicy, JudgeDecision,
    JudgeVerdict, PrdIdentity, TrustedCwd,
};

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

#[test]
fn named_existing_file_content_is_shown_and_judge_rule_is_updated() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("data")).unwrap();
    std::fs::write(
        root.path().join("data/input"),
        "asserts every component against its expected value\n",
    )
    .unwrap();
    let mut subject = contract();
    if let archon_workflow::task_set_contract::AcceptanceCheck::Command { command, .. } =
        &mut subject.acceptance[0].check
    {
        *command = "operation data/input".into();
    }
    let roots = JudgeRoots {
        project: Some(root.path().to_path_buf()),
        repository: None,
    };
    let prompt = batched_judge_prompt_with_roots(&subject, &roots).unwrap();
    assert!(prompt.contains("\"named_files\":[{\"content\":\"asserts every component against its expected value\\n\",\"path\":\"data/input\"}]"), "{prompt}");
    assert!(prompt.contains(NAMED_FILES_INSTRUCTION));
}

#[test]
fn checks_without_files_keep_their_entry_shape_and_retain_existing_prompt_bytes() {
    let mut subject = contract();
    if let archon_workflow::task_set_contract::AcceptanceCheck::Command { command, .. } =
        &mut subject.acceptance[0].check
    {
        *command = "true".into();
    }
    let original = batched_judge_prompt(&subject).unwrap();
    let root = tempfile::tempdir().unwrap();
    let rooted = batched_judge_prompt_with_roots(
        &subject,
        &JudgeRoots {
            project: Some(root.path().to_path_buf()),
            repository: None,
        },
    )
    .unwrap();
    let original_json = original.split_once("Checks: ").unwrap().1;
    let rooted_json = rooted.split_once("Checks: ").unwrap().1;
    assert_eq!(rooted_json, original_json);
}

#[test]
fn outside_and_missing_named_paths_are_omitted() {
    let root = tempfile::tempdir().unwrap();
    let outside = root.path().parent().unwrap().join("outside-input");
    std::fs::write(&outside, "outside").unwrap();
    let mut subject = contract();
    if let archon_workflow::task_set_contract::AcceptanceCheck::Command { command, .. } =
        &mut subject.acceptance[0].check
    {
        *command = "operation ../outside-input && operation missing-input".into();
    }
    let prompt = batched_judge_prompt_with_roots(
        &subject,
        &JudgeRoots {
            project: Some(root.path().to_path_buf()),
            repository: None,
        },
    )
    .unwrap();
    let checks: serde_json::Value =
        serde_json::from_str(prompt.split_once("Checks: ").unwrap().1).unwrap();
    assert!(checks[0].get("named_files").is_none(), "{checks}");
    std::fs::remove_file(outside).unwrap();
}

#[test]
fn named_file_content_is_truncated_at_existing_judged_source_limit() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("data")).unwrap();
    let limit = archon_workflow::check_source_settle::MAX_JUDGED_SOURCE_BYTES;
    std::fs::write(root.path().join("data/input"), vec![b'x'; limit + 1]).unwrap();
    let mut subject = contract();
    if let archon_workflow::task_set_contract::AcceptanceCheck::Command { command, .. } =
        &mut subject.acceptance[0].check
    {
        *command = "operation data/input".into();
    }
    let prompt = batched_judge_prompt_with_roots(
        &subject,
        &JudgeRoots {
            project: Some(root.path().to_path_buf()),
            repository: None,
        },
    )
    .unwrap();
    assert!(prompt.contains("[truncated at the existing judged-source byte limit]"));
    assert!(!prompt.contains(&"x".repeat(limit + 1)));
}
