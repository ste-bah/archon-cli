use super::*;
use archon_workflow::task_set_contract::{FreezeGateMode, JudgeDecision};

fn root_alias_fixture() -> (tempfile::TempDir, PathBuf, PathBuf, AcceptanceContract) {
    let temp = tempfile::tempdir().unwrap();
    let (tasks, input, bytes) = seed(&temp);
    std::fs::write(temp.path().join("input"), "ready\n").unwrap();
    let name = temp.path().file_name().unwrap().to_str().unwrap();
    let mut subject: AcceptanceContract = serde_json::from_slice(&bytes).unwrap();
    subject.acceptance[0].check = archon_workflow::task_set_contract::AcceptanceCheck::Command {
        command: format!("test -s ../{name}/input"),
        cwd: archon_workflow::task_set_contract::TrustedCwd::RepoRoot,
    };
    subject.acceptance[0].judgment.verdict = JudgeDecision::Accepted;
    (temp, tasks, input, subject)
}

#[test]
fn contract_policy_reports_an_own_root_alias_before_publication() {
    let (temp, _, _, subject) = root_alias_fixture();
    let findings = archon_workflow::task_set_contract::acceptance_policy_findings_with_roots(
        &subject,
        temp.path(),
        temp.path(),
        false,
    );
    assert!(
        findings.iter().any(|finding| {
            finding.field == "AC-X-001.check" && finding.message.contains("site does not create")
        }),
        "own-root path from {} had no policy finding: {findings:?}",
        temp.path().display()
    );
}

#[tokio::test]
async fn contract_policy_first_author_refuses_an_own_root_alias_before_judging() {
    let (temp, tasks, input, subject) = root_alias_fixture();
    let calls = Arc::new(AtomicUsize::new(0));
    let outcome = prepare_acceptance_freeze_from_candidate(
        temp.path(),
        &tasks,
        &input,
        GateMode::Observe,
        serde_json::to_vec(&subject).unwrap(),
        Arc::new(FinishReasonJudge {
            calls: calls.clone(),
            stop_reason: Some("end_turn".into()),
            content: r#"{"decisions":[{"id":"AC-X-001","verdict":"accepted","counterexample":"none","reason":"checks the input"}]}"#.into(),
        }),
    )
    .await;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "invalid path reached the judge"
    );
    let error = outcome.expect_err("the first author must repair the site path");
    assert!(CandidateRejected::caused(&error), "{error:#}");
    assert!(
        format!("{error:#}").contains("site does not create"),
        "{error:#}"
    );
}

#[test]
fn contract_policy_publication_records_an_own_root_alias_even_if_judged_accepted() {
    let (temp, tasks, input, subject) = root_alias_fixture();
    let text = std::fs::read_to_string(&input).unwrap();
    let prepared = findings::finish_acceptance(
        temp.path(),
        &tasks,
        &input,
        &text,
        FreezeGateMode::Observe,
        &subject,
        Vec::new(),
        None,
        "candidate".into(),
    )
    .unwrap();
    assert!(
        prepared.findings.iter().any(|finding| {
            finding.subject == "AC-X-001" && finding.text.contains("site does not create")
        }),
        "publication omitted the site-path refusal: {:?}",
        prepared.findings
    );
}
