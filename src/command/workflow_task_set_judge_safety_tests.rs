#![allow(non_snake_case)]
use super::super::continuation_tests::{Scripted, contract};
use super::tests::{git, reply};
use super::*;
use archon_workflow::task_set_contract::{AcceptanceCheck, TrustedCwd};

fn forwarded_after_reasks(kind: &str, state: &str, reason: &str) {
    let mut subject = contract();
    let mut screen = Screen::default();
    let content = reply(kind, state, reason, &[]);
    for attempt in 0..3 {
        assert_eq!(
            screen
                .apply(&mut subject, &content, &JudgeRoots::default())
                .unwrap()
                .is_empty(),
            attempt == 2
        );
        assert!(!screen.instruction(&subject).contains("or accept the check"));
    }
    assert_eq!(
        subject.acceptance[0].judgment.verdict,
        JudgeDecision::Refuted
    );
    assert_eq!(subject.acceptance[0].judgment.counterexample, state);
    assert!(
        subject.acceptance[0]
            .judgment
            .reason
            .contains(&format!("[kind={kind}]"))
    );
    assert_ne!(
        subject.acceptance[0].judgment.host_call_id,
        "host:refutation-without-state"
    );
}

#[test]
fn R6_R2_pure_state_denial_alone_can_be_accepted() {
    let mut subject = contract();
    let mut screen = Screen::default();
    let content = reply(
        "state",
        "No change is needed.",
        "No counterexample exists.",
        &[],
    );
    for attempt in 0..3 {
        assert_eq!(
            screen
                .apply(&mut subject, &content, &JudgeRoots::default())
                .unwrap()
                .is_empty(),
            attempt == 2
        );
    }
    assert_eq!(
        subject.acceptance[0].judgment.verdict,
        JudgeDecision::Accepted
    );
    for kind in ["other", "duplicate", "coverage"] {
        forwarded_after_reasks(kind, "No counterexample exists.", "No change is needed.");
    }
}

#[test]
fn R6_R2_mixed_denials_preserve_constructive_states() {
    for state in [
        "No counterexample via PATH stubbing is valid; but an implementation returning an empty list passes.",
        "No counterexample through the move-aside handle. The test only asserts length, so wrong values pass.",
    ] {
        assert!(!denies_state(state));
        forwarded_after_reasks(
            "state",
            state,
            "The deciding assertion misses incorrect values.",
        );
    }
    forwarded_after_reasks(
        "state",
        "No counterexample exists.",
        "No change is needed to the harness, but an empty result passes its assertion.",
    );
}

#[test]
fn R6_R2_content_duplicate_is_never_host_accepted() {
    forwarded_after_reasks(
        "state",
        "A relocated implementation bypasses the assertion.",
        "The test checks an old copy.",
    );
    forwarded_after_reasks(
        "duplicate",
        "A delivered duplicate bypasses the assertion.",
        "No existing path was supplied.",
    );
}

#[test]
fn R6_R2_toolchain_and_stale_claims_are_never_host_accepted() {
    for state in [
        "A shell quirk changes the command status.",
        "A cached binary hides the changed source.",
    ] {
        forwarded_after_reasks("other", state, "The claimed state passes the check.");
    }
}

#[test]
fn R6_grep_deciding_a_duplicated_gate_is_forwarded() {
    forwarded_after_reasks(
        "state",
        "The check greps for identifier presence in a duplicated gate rather than asserting its behaviour; a wrong result passes.",
        "A grep decides by matching text in a source file.",
    );
}

#[test]
fn R6_duplicate_rows_is_a_state_not_a_duplicate_claim() {
    let mut subject = contract();
    let state = "An implementation that writes duplicate rows passes the deciding test.";
    assert!(!duplicate_claim(state));
    assert!(
        Screen::default()
            .apply(
                &mut subject,
                &reply("state", state, "The test never checks uniqueness.", &[]),
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

fn duplicate_fixture(command: &str, cwd: TrustedCwd) -> (tempfile::TempDir, JudgeRoots) {
    let parent = tempfile::tempdir().unwrap();
    let repository = parent.path().join("repository");
    let project = parent.path().join("project");
    std::fs::create_dir(&repository).unwrap();
    std::fs::create_dir(&project).unwrap();
    git(&repository, &["init", "-q"]);
    let check = AcceptanceCheck::Command {
        command: command.into(),
        cwd,
    };
    std::fs::write(
        repository.join("existing.json"),
        serde_json::to_vec(&serde_json::json!({"check": check})).unwrap(),
    )
    .unwrap();
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
        repository: Some(repository),
        project: Some(project),
        repository_commit: Some("HEAD".into()),
        ..Default::default()
    };
    (parent, roots)
}

#[test]
fn R6_R2_identical_duplicate_command_and_cwd_is_accepted() {
    let (_parent, roots) = duplicate_fixture("run --exact\r\n", TrustedCwd::ProjectRoot);
    let mut subject = contract();
    subject.acceptance[0].check = AcceptanceCheck::Command {
        command: "run --exact\n".into(),
        cwd: TrustedCwd::ProjectRoot,
    };
    let mut screen = Screen::default();
    let content = reply(
        "duplicate",
        "Another check duplicates this command.",
        "The existing check is equivalent.",
        &["../repository/./existing.json"],
    );
    for attempt in 0..3 {
        assert_eq!(
            screen
                .apply(&mut subject, &content, &roots)
                .unwrap()
                .is_empty(),
            attempt == 2
        );
    }
    assert_eq!(
        subject.acceptance[0].judgment.verdict,
        JudgeDecision::Accepted
    );
    assert_eq!(
        subject.acceptance[0].judgment.host_call_id,
        "host:identical-duplicate-check"
    );
}

#[test]
fn R6_R2_duplicate_substrings_and_different_cwds_are_forwarded() {
    for (existing, cwd, kind) in [
        ("run --exact && true", TrustedCwd::RepoRoot, "duplicate"),
        ("run --exact", TrustedCwd::ProjectRoot, "duplicate"),
        ("run --exact", TrustedCwd::RepoRoot, "state"),
    ] {
        let (_parent, roots) = duplicate_fixture(existing, cwd);
        let mut subject = contract();
        subject.acceptance[0].check = AcceptanceCheck::Command {
            command: "run --exact".into(),
            cwd: TrustedCwd::RepoRoot,
        };
        let content = reply(
            kind,
            "The delivered duplicate has no assertion.",
            "The existing check is equivalent.",
            &["existing.json"],
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
        assert_eq!(
            subject.acceptance[0].judgment.verdict,
            JudgeDecision::Refuted
        );
    }
}

#[tokio::test]
async fn R6_R3_coverage_prefetch_does_not_spend_a_fault_reask() {
    let coverage = reply(
        "coverage",
        "The bound is unasserted.",
        "The test misses the bound.",
        &["test.rs"],
    );
    let stale = reply(
        "other",
        "A cached binary hides the changed source.",
        "The stale build passes.",
        &[],
    );
    let client = Scripted::new(vec![
        Ok((&coverage, Some("end_turn"))),
        Ok((&stale, Some("end_turn"))),
        Ok((&stale, Some("end_turn"))),
        Ok((&stale, Some("end_turn"))),
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
    assert_eq!(client.calls().len(), 4);
    assert_eq!(
        judged.acceptance[0].judgment.verdict,
        JudgeDecision::Refuted
    );
}

#[test]
fn R6_R4_stale_prebuilt_refutation_survives_and_fact_matches_prompt() {
    let prompt = batched_judge_prompt(&contract()).unwrap();
    assert!(prompt.contains(TOOLCHAIN_FACT));
    forwarded_after_reasks(
        "state",
        "A stale build product predates the required behaviour.",
        "The check runs a prebuilt binary without building it.",
    );
}

#[test]
fn R6_R5_passability_fault_names_only_its_real_schema() {
    let error =
        apply_unscreened(&mut contract(), r#"{"decisions":[{"id":"AC-X-001"}]}"#).unwrap_err();
    let text = format!("{error:#}");
    assert!(text.contains("id, verdict, counterexample, reason"));
    assert!(!text.contains("kind"));
    assert!(!text.contains("cited_paths"));
}

#[test]
fn R6_R7_normal_replies_do_no_additional_source_reads_or_commit_resolution() {
    let mut screen = Screen::default();
    let mut subject = contract();
    let before = body_reads();
    screen
        .apply(
            &mut subject,
            &reply(
                "state",
                "An empty result passes.",
                "No assertion checks its values.",
                &[],
            ),
            &JudgeRoots::default(),
        )
        .unwrap();
    assert_eq!(body_reads(), before);
    assert!(screen.resolved_roots.is_none());
    let accepted = reply("other", "None found.", "The assertion is sound.", &[])
        .replace("\"refuted\"", "\"accepted\"");
    screen
        .apply(&mut subject, &accepted, &JudgeRoots::default())
        .unwrap();
    assert_eq!(body_reads(), before);
    assert!(screen.resolved_roots.is_none());
}
