#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::Ordering::SeqCst;

use super::*;
use crate::command::workflow_task_set::passability_tests::EvidenceJudge;

fn fixture(parser_error: bool) -> (tempfile::TempDir, AcceptanceContract, BaselineRuns) {
    let external = tempfile::tempdir().unwrap();
    let executable = external.path().join("tool");
    std::fs::write(&executable, "#!/bin/sh\nexit 2\n").unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    let acceptance = ["AC-X-001", "AC-X-002"].map(|id| {
        serde_json::json!({
            "id": id, "criterion": "output is valid",
            "check": {"kind": "command", "command": "tool --new-flag", "cwd": "project_root"},
            "judgment": {"verdict": "accepted", "counterexample": "", "reason": "", "host_call_id": ""}
        })
    });
    let contract = serde_json::from_value(serde_json::json!({
        "schema_version": 1,
        "prd": {"path": "fixture.md", "digest": "fixture"},
        "gap_policy": {"permitted_acceptance_ids": [], "forbidden_phrases": [], "required_fields": []},
        "acceptance": acceptance,
        "supplementary": []
    }))
    .unwrap();
    let stderr = if parser_error {
        "error: unexpected argument '--new-flag' found\nUsage: tool [OPTIONS]\n"
    } else {
        "gate refused: below required minimum\n"
    };
    let result = archon_workflow::acceptance_scratch::CheckResult {
        acceptance_id: "AC-X-002".into(),
        exit_code: Some(2),
        stderr: stderr.as_bytes().to_vec(),
        stdout: Vec::new(),
        verdict_streams: Default::default(),
        fault: None,
        quota_walk_count: 0,
        environment_note: None,
        operational_error: None,
        classification: None,
    };
    let runs = BaselineRuns {
        commit: "evidence-tree".into(),
        failures: BTreeMap::from([(
            "AC-X-002".into(),
            super::super::super::executability::CheckEvidence {
                result,
                copy_layout: None,
            },
        )]),
        environment: BTreeMap::from([("PATH".into(), external.path().display().to_string())]),
        forwarded: Vec::new(),
        reuse: Default::default(),
    };
    (external, contract, runs)
}

fn probe_finding(tasks: &Path) -> GateFinding {
    GateFinding::new(
        GateId::FreezeAcceptance,
        "check 'AC-X-001' already holds on the evidence tree",
        "AC-X-001",
        Some(tasks.join(ACCEPTANCE_CONTRACT_FILE)),
        archon_workflow::RemediationScope::CandidateArtifact,
    )
}

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

#[tokio::test]
async fn an_author_probe_finding_still_defers_model_passability_judging() {
    let (_external, mut contract, runs) = fixture(false);
    let root = tempfile::tempdir().unwrap();
    let client = EvidenceJudge::default();
    let findings = judge_baseline_failures(
        root.path(),
        root.path(),
        root.path(),
        "",
        &client,
        &mut contract,
        PreImplementation {
            findings: vec![probe_finding(root.path())],
            baseline: Some(runs),
            host_unproven_reason: None,
        },
        &FreezeResume::none(),
    )
    .await
    .unwrap();
    assert_eq!(findings.len(), 1);
    assert_eq!(
        contract.acceptance[1].judgment.verdict,
        JudgeDecision::Accepted
    );
    assert_eq!(client.evidence.load(SeqCst), 0);
    assert_eq!(client.plain.load(SeqCst), 0);
}

#[tokio::test]
async fn without_probe_findings_parser_and_model_refutations_are_unchanged() {
    for parser_error in [true, false] {
        let (_external, mut contract, runs) = fixture(parser_error);
        let root = tempfile::tempdir().unwrap();
        let client = EvidenceJudge::default();
        let findings = judge_baseline_failures(
            root.path(),
            root.path(),
            root.path(),
            "",
            &client,
            &mut contract,
            PreImplementation {
                findings: Vec::new(),
                baseline: Some(runs),
                host_unproven_reason: None,
            },
            &FreezeResume::none(),
        )
        .await
        .unwrap();
        assert_eq!(findings.len(), 1, "{:?}", findings);
        assert_eq!(findings[0].subject, "AC-X-002");
        assert!(
            findings[0]
                .text
                .starts_with(&format!("check 'AC-X-002': {CANNOT_PASS}"))
        );
        assert_eq!(
            contract.acceptance[1].judgment.verdict,
            JudgeDecision::Refuted
        );
        assert_eq!(client.evidence.load(SeqCst), usize::from(!parser_error));
    }
}
