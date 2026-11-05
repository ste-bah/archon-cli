#![allow(non_snake_case)]
use super::super::continuation_tests::{Scripted, contract};
use super::tests::{git, reply};
use super::*;
use archon_workflow::task_set_contract::{AcceptanceCheck, TrustedCwd};

#[tokio::test]
async fn R1_a_missing_schema_reasks_with_keys_and_allowed_kinds() {
    let old = r#"{"decisions":[{"id":"AC-X-001","verdict":"accepted","reason":"sound","counterexample":"none"}]}"#;
    let good = reply("other", "none", "sound", &[]).replace("\"refuted\"", "\"accepted\"");
    let client = Scripted::new(vec![
        Ok((old, Some("end_turn"))),
        Ok((&good, Some("end_turn"))),
    ]);
    let judged = judge_batch(
        &client,
        contract(),
        "sonnet",
        require_judged_prose,
        None,
        &JudgeRoots::default(),
    )
    .await
    .unwrap();
    assert_eq!(
        judged.acceptance[0].judgment.verdict,
        JudgeDecision::Accepted
    );
    let calls = client.calls();
    assert_eq!(calls.len(), 2);
    let error = calls[1].last().unwrap()["content"].as_str().unwrap();
    for text in ["kind", "cited_paths", "state, duplicate, coverage, other"] {
        assert!(error.contains(text), "{error}");
    }
}

#[test]
fn R1_c_fresh_schema_enters_the_screen() {
    let mut subject = contract();
    let content = reply(
        "state",
        "An empty result passes.",
        "No assertion checks the result.",
        &[],
    );
    assert!(
        Screen::default()
            .apply(&mut subject, &content, &JudgeRoots::default())
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        subject.acceptance[0].judgment.verdict,
        JudgeDecision::Refuted
    );
    let accepted = content.replace("\"refuted\"", "\"accepted\"");
    let mut accepted_subject = contract();
    assert!(
        Screen::default()
            .apply(&mut accepted_subject, &accepted, &JudgeRoots::default())
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        accepted_subject.acceptance[0].judgment.verdict,
        JudgeDecision::Accepted
    );
    for key in ["kind", "cited_paths"] {
        let mut missing: serde_json::Value = serde_json::from_str(&content).unwrap();
        missing["decisions"][0].as_object_mut().unwrap().remove(key);
        assert!(
            Screen::default()
                .apply(
                    &mut contract(),
                    &missing.to_string(),
                    &JudgeRoots::default()
                )
                .is_err()
        );
    }
}

#[test]
fn R2_prompt_defines_kinds_and_path_root() {
    let prompt = batched_judge_prompt(&contract()).unwrap();
    for text in [
        "state describes",
        "duplicate claims",
        "coverage claims",
        "other covers",
        "For accepted decisions use other",
        "relative to the check working directory",
        "screened by content even if labelled state",
    ] {
        assert!(prompt.contains(text), "missing {text}");
    }
}

#[test]
fn R3_a_constructive_refutations_survive_denial_wording() {
    for state in [
        "The check runs no assertion on the cap field; a state where the cap is ignored passes, so a direct cap test is needed.",
        "Moving the file aside is not a counterexample; an implementation returning an empty list passes.",
    ] {
        let mut subject = contract();
        let mut screen = Screen::default();
        for _ in 0..3 {
            screen
                .apply(
                    &mut subject,
                    &reply("state", state, "The deciding assertion is absent.", &[]),
                    &JudgeRoots::default(),
                )
                .unwrap();
        }
        assert_eq!(
            subject.acceptance[0].judgment.verdict,
            JudgeDecision::Refuted
        );
    }
    for denial in [
        "No valid counterexample exists.",
        "No counterexample is needed.",
        "No in-bounds state is constructible.",
        "Not a counterexample.",
    ] {
        assert!(denies_state(denial));
        assert!(
            !Screen::default()
                .apply(
                    &mut contract(),
                    &reply("state", denial, "unsound", &[]),
                    &JudgeRoots::default()
                )
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn R3_b_project_relative_duplicate_resolves_in_base() {
    let parent = tempfile::tempdir().unwrap();
    let project = parent.path().join("project");
    let repository = parent.path().join("repository");
    std::fs::create_dir(&project).unwrap();
    std::fs::create_dir(&repository).unwrap();
    git(&repository, &["init", "-q"]);
    std::fs::write(repository.join("copy.rs"), "fn main() {}\n").unwrap();
    git(&repository, &["add", "."]);
    git(
        &repository,
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
        project: Some(project),
        repository: Some(repository),
        repository_commit: Some("HEAD".into()),
        ..Default::default()
    };
    let mut subject = contract();
    subject.acceptance[0].check = AcceptanceCheck::Command {
        command: "run".into(),
        cwd: TrustedCwd::ProjectRoot,
    };
    for path in [
        "../repository/copy.rs",
        "../repository/missing.rs",
        "../../outside.rs",
    ] {
        let mut checked = subject.clone();
        assert_eq!(
            named_files::exists_in_base(path, &checked.acceptance[0], &roots),
            path.ends_with("/copy.rs")
        );
        let mut screen = Screen::default();
        let content = reply(
            "duplicate",
            "The delivered duplicate bypasses the assertion.",
            "The check checks an old copy.",
            &[path],
        );
        screen.apply(&mut checked, &content, &roots).unwrap();
        screen.apply(&mut checked, &content, &roots).unwrap();
        let missing = screen
            .apply(
                &mut checked,
                &reply(
                    "duplicate",
                    "The delivered duplicate bypasses the assertion.",
                    "The check checks an old copy.",
                    &[path],
                ),
                &roots,
            )
            .unwrap();
        assert!(missing.is_empty());
        if path.ends_with("/copy.rs") {
            assert!(author_note(&checked.acceptance[0]).contains(path));
            assert_eq!(
                checked.acceptance[0].judgment.verdict,
                JudgeDecision::Refuted
            );
        }
    }
}

#[test]
fn R3_c_content_screens_mislabelled_claims_and_preserves_real_states() {
    for (state, reason, expected) in [
        (
            "A cached binary hides the changed source.",
            "The stale build passes.",
            TOOLCHAIN_FACT,
        ),
        (
            "The delivered duplicate has no assertion.",
            "The test checks an old copy.",
            "name a path that exists",
        ),
    ] {
        let mut screen = Screen::default();
        let mut subject = contract();
        assert!(
            !screen
                .apply(
                    &mut subject,
                    &reply("state", state, reason, &[]),
                    &JudgeRoots::default()
                )
                .unwrap()
                .is_empty()
        );
        assert!(screen.instruction(&subject).contains(expected));
    }
    let mut subject = contract();
    assert!(
        Screen::default()
            .apply(
                &mut subject,
                &reply(
                    "state",
                    "The bound is ignored.",
                    "The assertion misses the bound.",
                    &[]
                ),
                &JudgeRoots::default()
            )
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        subject.acceptance[0].judgment.verdict,
        JudgeDecision::Refuted
    );
}
