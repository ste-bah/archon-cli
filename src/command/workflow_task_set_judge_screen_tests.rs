#![allow(non_snake_case)]
use super::super::continuation_tests::{Scripted, contract};
use super::*;
use archon_workflow::task_set_contract::{AcceptanceCheck, TrustedCwd};

pub(super) fn reply(kind: &str, counterexample: &str, reason: &str, paths: &[&str]) -> String {
    serde_json::json!({"decisions":[{"id":"AC-X-001", "verdict":"refuted",
        "kind":kind, "cited_paths":paths, "counterexample":counterexample, "reason":reason}]})
    .to_string()
}

#[test]
fn P4_1_state_is_forwarded_unchanged_and_capped() {
    let mut subject = contract();
    let reason = "r".repeat(450);
    let state = "The implementation returns an empty result for a required record.";
    assert!(
        Screen::default()
            .apply(
                &mut subject,
                &reply("state", state, &reason, &[]),
                &JudgeRoots::default()
            )
            .unwrap()
            .is_empty()
    );
    assert_eq!(subject.acceptance[0].judgment.counterexample, state);
    assert_eq!(subject.acceptance[0].judgment.reason, reason);
    let note = author_note(&subject.acceptance[0]);
    assert!(note.contains(&format!("{}…", "r".repeat(400))));
    assert!(!note.contains(&"r".repeat(401)));
}

#[tokio::test]
async fn P4_2_contradiction_reasks_only_its_id() {
    let mut subject = contract();
    let mut second = subject.acceptance[0].clone();
    second.id = "AC-X-002".into();
    subject.acceptance.push(second);
    let mut third = subject.acceptance[0].clone();
    third.id = "AC-X-003".into();
    subject.acceptance.push(third);
    let first = serde_json::json!({"decisions":[
        {"id":"AC-X-001","verdict":"refuted","kind":"state","cited_paths":[],"counterexample":"No in-bounds state is constructible.","reason":"none is constructible"},
        {"id":"AC-X-002","verdict":"accepted","kind":"other","cited_paths":[],"counterexample":"none","reason":"sound"},
        {"id":"AC-X-003","verdict":"refuted","kind":"state","cited_paths":[],"counterexample":"No counterexample is needed.","reason":"No state is constructible."}
    ]}).to_string();
    let second = reply(
        "state",
        "The implementation returns an empty record.",
        "The assertion misses it.",
        &[],
    );
    let third = second.replace("AC-X-001", "AC-X-003");
    let client = Scripted::new(vec![
        Ok((&first, Some("end_turn"))),
        Ok((&second, Some("end_turn"))),
        Ok((&third, Some("end_turn"))),
    ]);
    let judged = judge_batch(
        &client,
        subject,
        "sonnet",
        require_judged_prose,
        None,
        &JudgeRoots::default(),
    )
    .await
    .unwrap();
    assert_eq!(
        judged.acceptance[0].judgment.verdict,
        JudgeDecision::Refuted
    );
    let calls = client.calls();
    assert_eq!(calls.len(), 3);
    let ask = calls[1][0]["content"].as_str().unwrap();
    assert!(ask.contains("contradicts its own evidence"));
    assert!(ask.contains("AC-X-001"));
    assert!(!ask.contains("AC-X-002"));
    assert!(!ask.contains("AC-X-003"));
    let next = calls[2][0]["content"].as_str().unwrap();
    assert!(next.contains("AC-X-003"));
    assert!(!next.contains("AC-X-001"));
    assert!(!next.contains("AC-X-002"));
    for denial in [
        "none is constructible",
        "no counterexample",
        "no in-bounds state",
        "not a counterexample",
        "no counterexample is needed",
    ] {
        assert!(denies_state(denial));
    }
}

#[test]
fn P4_3_second_reask_records_host_acceptance() {
    let mut screen = Screen::default();
    let mut subject = contract();
    let content = reply(
        "state",
        "No in-bounds state is constructible.",
        "No counterexample is needed.",
        &[],
    );
    for _ in 0..2 {
        assert_eq!(
            screen
                .apply(&mut subject, &content, &JudgeRoots::default())
                .unwrap(),
            BTreeSet::from(["AC-X-001".into()])
        );
    }
    assert!(
        screen
            .apply(&mut subject, &content, &JudgeRoots::default())
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        subject.acceptance[0].judgment.verdict,
        JudgeDecision::Accepted
    );
    assert_eq!(
        subject.acceptance[0].judgment.host_call_id,
        "host:refutation-without-state"
    );
}

#[test]
fn P4_4_duplicate_requires_existing_base_path() {
    for paths in [vec![], vec!["missing.rs"]] {
        let mut screen = Screen::default();
        let mut subject = contract();
        let content = reply(
            "duplicate",
            "A duplicate is delivered.",
            "The test checks another copy.",
            &paths,
        );
        assert!(
            !screen
                .apply(&mut subject, &content, &JudgeRoots::default())
                .unwrap()
                .is_empty()
        );
        assert!(
            screen
                .instruction(&subject)
                .contains("name a path that exists")
        );
        screen
            .apply(&mut subject, &content, &JudgeRoots::default())
            .unwrap();
        screen
            .apply(&mut subject, &content, &JudgeRoots::default())
            .unwrap();
        assert_eq!(
            subject.acceptance[0].judgment.verdict,
            JudgeDecision::Refuted
        );
        assert!(
            subject.acceptance[0]
                .judgment
                .reason
                .contains("[kind=duplicate]")
        );
    }
}

pub(super) fn git(root: &Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn P4_5_existing_duplicate_names_path_in_finding() {
    let root = tempfile::tempdir().unwrap();
    git(root.path(), &["init", "-q"]);
    std::fs::write(root.path().join("duplicate.rs"), "fn main() {}\n").unwrap();
    std::fs::create_dir(root.path().join("copies")).unwrap();
    std::fs::write(root.path().join("copies/duplicate.rs"), "fn main() {}\n").unwrap();
    git(root.path(), &["add", "."]);
    git(
        root.path(),
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-qm",
            "base",
        ],
    );
    let roots = JudgeRoots {
        project: Some(root.path().into()),
        repository: Some(root.path().into()),
        repository_commit: Some("HEAD".into()),
        ..Default::default()
    };
    assert!(named_files::exists_in_base(
        "copies",
        &contract().acceptance[0],
        &roots
    ));
    let mut subject = contract();
    let content = reply(
        "duplicate",
        "The delivered copy omits the assertion.",
        "The test checks another copy.",
        &["duplicate.rs"],
    );
    let mut screen = Screen::default();
    for attempt in 0..3 {
        assert_eq!(
            screen
                .apply(&mut subject, &content, &roots)
                .unwrap()
                .is_empty(),
            attempt == 2
        );
    }
    assert!(author_note(&subject.acceptance[0]).contains("duplicate.rs"));
}

#[test]
fn P4_6_coverage_prefetches_then_forwards() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("test.rs"), "assert!(actual > 0);\n").unwrap();
    let roots = JudgeRoots {
        project: Some(root.path().into()),
        task_root: Some(root.path().into()),
        ..Default::default()
    };
    let content = reply(
        "coverage",
        "The test never checks the upper bound.",
        "A covered bound is unasserted.",
        &["test.rs"],
    );
    let mut subject = contract();
    let mut screen = Screen::default();
    assert!(
        !screen
            .apply(&mut subject, &content, &roots)
            .unwrap()
            .is_empty()
    );
    assert!(screen.instruction(&subject).contains("assert!(actual > 0)"));
    assert!(
        screen
            .apply(&mut subject, &content, &roots)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        subject.acceptance[0].judgment.verdict,
        JudgeDecision::Refuted
    );
}

#[test]
fn P4_7_supplied_coverage_is_forwarded() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("test.rs"), "assert!(actual > 0);\n").unwrap();
    let roots = JudgeRoots {
        project: Some(root.path().into()),
        task_root: Some(root.path().into()),
        ..Default::default()
    };
    let mut subject = contract();
    subject.acceptance[0].check = AcceptanceCheck::Command {
        command: "cat test.rs".into(),
        cwd: TrustedCwd::ProjectRoot,
    };
    let content = reply(
        "coverage",
        "The upper bound is unasserted.",
        "The test misses the bound.",
        &["test.rs"],
    );
    assert!(
        Screen::default()
            .apply(&mut subject, &content, &roots)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn P4_8_toolchain_claim_reasks_with_fact() {
    let mut subject = contract();
    let mut screen = Screen::default();
    let content = reply(
        "other",
        "A cached binary is reused.",
        "The stale build hides the change.",
        &[],
    );
    assert!(
        !screen
            .apply(&mut subject, &content, &JudgeRoots::default())
            .unwrap()
            .is_empty()
    );
    assert!(screen.instruction(&subject).contains(TOOLCHAIN_FACT));
}

#[test]
fn P4_9_accepted_is_unchanged_for_any_kind() {
    for kind in ["state", "duplicate", "coverage", "other"] {
        let content = reply(kind, "No counterexample is constructible.", "sound", &[])
            .replace("\"refuted\"", "\"accepted\"");
        let mut subject = contract();
        assert!(
            Screen::default()
                .apply(&mut subject, &content, &JudgeRoots::default())
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            subject.acceptance[0].judgment.host_call_id,
            "acceptance-judge-batch:AC-X-001"
        );
        assert_eq!(subject.acceptance[0].judgment.reason, "sound");
    }
}

#[tokio::test]
async fn P4_10_missing_kind_is_unusable() {
    let content = r#"{"decisions":[{"id":"AC-X-001","verdict":"accepted","counterexample":"none","reason":"sound","cited_paths":[]}]}"#;
    let mut subject = contract();
    let before = subject.acceptance[0].judgment.host_call_id.clone();
    assert!(
        Screen::default()
            .apply(&mut subject, content, &JudgeRoots::default())
            .is_err()
    );
    assert_eq!(subject.acceptance[0].judgment.host_call_id, before);
    let mut batch = contract();
    let mut second = batch.acceptance[0].clone();
    second.id = "AC-X-002".into();
    batch.acceptance.push(second);
    let good = reply("state", "none", "sound", &[]).replace("\"refuted\"", "\"accepted\"");
    let mut good_json: serde_json::Value = serde_json::from_str(&good).unwrap();
    let mut second = good_json["decisions"][0].clone();
    second["id"] = serde_json::json!("AC-X-002");
    good_json["decisions"].as_array_mut().unwrap().push(second);
    let good = good_json.to_string();
    let mut old = good_json.clone();
    old["decisions"][0].as_object_mut().unwrap().remove("kind");
    let old = old.to_string();
    let client = Scripted::new(vec![
        Ok((&old, Some("end_turn"))),
        Ok((&good, Some("end_turn"))),
    ]);
    let judged = judge_batch(
        &client,
        batch,
        "sonnet",
        require_judged_prose,
        None,
        &JudgeRoots::default(),
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
    let task = calls[1][0]["content"].as_str().unwrap();
    assert!(task.contains("AC-X-001") && task.contains("AC-X-002"));
    assert!(
        calls[1].last().unwrap()["content"]
            .as_str()
            .unwrap()
            .contains("missing field `kind`")
    );
    // Passability retains its existing schema independently of the main judge.
    assert!(apply_unscreened(&mut subject, content).is_ok());
    for malformed in ["42", r#"{"decisions":[42]}"#, "not JSON"] {
        assert!(apply_unscreened(&mut subject, malformed).is_err());
    }
}

#[test]
fn P4_11_forwarded_finding_keeps_existing_form() {
    assert_eq!(
        archon_workflow::task_set_contract::refuted_check_message(
            "X",
            "bad\nstate",
            "a false result"
        ),
        "check 'X' was refuted by the host judge; reason: \"bad state\"; counterexample: \"a false result\"; replace the check with one that fails in that state"
    );
}
