use super::*;

#[tokio::test]
async fn an_author_probe_finding_does_not_withhold_another_checks_parser_refutation() {
    let (_external, mut contract, runs) = fixture(true);
    let root = tempfile::tempdir().unwrap();
    let tasks = root.path().join("tasks");
    let original = probe_finding(&tasks);
    let original_text = original.text.clone();
    let client = EvidenceJudge::default();
    let findings = judge_baseline_failures(
        root.path(),
        root.path(),
        &tasks,
        "",
        &client,
        &mut contract,
        PreImplementation {
            baseline_outcomes: Default::default(),
            findings: vec![original],
            baseline: Some(runs),
            host_unproven_reason: None,
        },
        &FreezeResume::saving(
            crate::command::workflow_freeze_budget::FreezeBudget::unlimited(),
            true,
        ),
    )
    .await
    .unwrap();
    assert_eq!(findings.len(), 2, "{:?}", findings);
    assert_eq!(findings[0].text, original_text);
    assert_eq!(findings[1].subject, "AC-X-002");
    assert!(
        findings[1]
            .text
            .starts_with(&format!("check 'AC-X-002': {CANNOT_PASS}"))
    );
    assert!(
        findings[1]
            .text
            .contains("outside the repository) rejected the check's command line")
    );
    assert!(
        findings[1]
            .text
            .contains("unexpected argument '--new-flag' found")
    );
    assert!(findings[1].text.contains("stderr:"));
    assert_eq!(
        findings[1].remediation_scope,
        archon_workflow::RemediationScope::CandidateArtifact
    );
    let judgment = &contract.acceptance[1].judgment;
    assert_eq!(judgment.verdict, JudgeDecision::Refuted);
    assert!(judgment.reason.starts_with(CANNOT_PASS));
    assert_eq!(
        judgment.counterexample,
        "error: unexpected argument '--new-flag' found"
    );
    assert_eq!(judgment.host_call_id, "host:argument-parser-usage");
    // #562: the passability stage, after the probe's own finding.
    assert_eq!(findings[0].gate_stage, None);
    assert_eq!(
        findings[1].gate_stage,
        Some(crate::command::workflow_gate::GateStage::Passability)
    );
    assert!(super::is_passability_call(&judgment.host_call_id));
    assert_eq!(
        contract.acceptance[0].judgment.verdict,
        JudgeDecision::Accepted
    );
    assert_eq!(client.evidence.load(SeqCst), 0);
    assert_eq!(client.plain.load(SeqCst), 0);
    let records = std::fs::read_dir(root.path().join(FREEZE_CACHE_DIR).join("passability"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(records.len(), 1);
    let record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&records[0]).unwrap()).unwrap();
    assert_eq!(record["check_id"], "AC-X-002");
    assert_eq!(record["verdict"], "refuted");
    assert_eq!(record["baseline_exit_code"], 2);
}

#[tokio::test]
async fn a_check_with_its_own_probe_finding_gets_no_second_cannot_pass_finding() {
    let (_external, mut contract, runs) = fixture(true);
    let root = tempfile::tempdir().unwrap();
    let mut own = probe_finding(root.path());
    own.subject = "AC-X-002".into();
    let own_text = own.text.clone();
    let client = EvidenceJudge::default();
    let findings = judge_baseline_failures(
        root.path(),
        root.path(),
        root.path(),
        "",
        &client,
        &mut contract,
        PreImplementation {
            baseline_outcomes: Default::default(),
            findings: vec![own],
            baseline: Some(runs),
            host_unproven_reason: None,
        },
        &FreezeResume::none(),
    )
    .await
    .unwrap();
    assert_eq!(findings.len(), 1, "{:?}", findings);
    assert_eq!(findings[0].text, own_text);
    assert_eq!(
        contract.acceptance[1].judgment.verdict,
        JudgeDecision::Accepted
    );
    assert_eq!(client.evidence.load(SeqCst), 0);
    assert_eq!(client.plain.load(SeqCst), 0);
}
