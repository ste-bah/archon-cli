use std::path::PathBuf;

use archon_cognitive::{
    CandidateActionKind, ContractInput, VerificationEngine, VerificationEvidence, VerificationKind,
    VerificationVerdict,
};

fn input(kind: VerificationKind) -> ContractInput {
    ContractInput {
        verification_kind: kind,
        action_kind: CandidateActionKind::InspectFiles,
        files_touched: vec![PathBuf::from("src/lib.rs")],
        commands_planned: Vec::new(),
        working_directory: PathBuf::from("."),
        situation_id: "situation-1".into(),
        override_reason: Some("human approved quality-gate continuation".into()),
    }
}

#[test]
fn code_edit_requires_test_evidence_per_file() {
    let contract = VerificationEngine
        .require(&input(VerificationKind::CodeEdit))
        .unwrap();

    assert_eq!(contract.requirements.len(), 1);
    assert_eq!(contract.requirements[0].evidence_type, "test_run");
    assert!(contract.requirements[0].fallback_if_unavailable.is_some());
}

#[test]
fn code_edit_without_files_fails_closed() {
    let mut input = input(VerificationKind::CodeEdit);
    input.files_touched.clear();

    let error = VerificationEngine.require(&input).unwrap_err().to_string();

    assert!(error.contains("no files touched"));
}

#[test]
fn commit_requires_status_and_diff_without_fallback() {
    let contract = VerificationEngine
        .require(&input(VerificationKind::Commit))
        .unwrap();
    let evidence_types = contract
        .requirements
        .iter()
        .map(|requirement| requirement.evidence_type.as_str())
        .collect::<Vec<_>>();

    assert_eq!(evidence_types, ["git_status", "git_diff"]);
    assert!(
        contract
            .requirements
            .iter()
            .all(|item| item.fallback_if_unavailable.is_none())
    );
}

#[test]
fn ci_debug_contract_mentions_log_lines_not_memory_guessing() {
    let contract = VerificationEngine
        .require(&input(VerificationKind::CiDebug))
        .unwrap();

    assert_eq!(contract.requirements[0].evidence_type, "log_evidence");
    assert!(
        contract.requirements[0]
            .acceptance_criteria
            .contains("line references")
    );
    assert!(
        contract.requirements[0]
            .acceptance_criteria
            .contains("memory")
    );
}

#[test]
fn verification_evidence_pass_fail_skip_and_not_run() {
    let engine = VerificationEngine;
    let commit = engine.require(&input(VerificationKind::Commit)).unwrap();
    let passed = vec![
        evidence("git_status", "repository working tree", Some(true)),
        evidence("git_diff", "staged or unstaged changes", Some(true)),
    ];
    assert_eq!(
        engine.verify_evidence(&commit, &passed),
        VerificationVerdict::Passed
    );

    let failed = vec![evidence(
        "git_status",
        "repository working tree",
        Some(false),
    )];
    assert!(matches!(
        engine.verify_evidence(&commit, &failed),
        VerificationVerdict::Failed { .. }
    ));

    let ci = engine.require(&input(VerificationKind::CiDebug)).unwrap();
    assert!(matches!(
        engine.verify_evidence(&ci, &[]),
        VerificationVerdict::Skipped { .. }
    ));

    let mut empty = ci.clone();
    empty.requirements.clear();
    assert_eq!(
        engine.verify_evidence(&empty, &[]),
        VerificationVerdict::NotRun
    );
}

#[test]
fn docs_update_verify_checks_path_existence() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("doc.md"), "ok").unwrap();
    let mut input = input(VerificationKind::DocsUpdate);
    input.files_touched = vec![PathBuf::from("doc.md")];
    let contract = VerificationEngine.require(&input).unwrap();

    assert_eq!(
        VerificationEngine.verify(&contract, dir.path()),
        VerificationVerdict::Passed
    );
}

fn evidence(evidence_type: &str, target: &str, passed: Option<bool>) -> VerificationEvidence {
    VerificationEvidence {
        evidence_type: evidence_type.into(),
        target: target.into(),
        passed,
        details: "test evidence".into(),
    }
}
