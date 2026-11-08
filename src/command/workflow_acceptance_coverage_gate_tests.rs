//! H4 / A12 freeze findings, and the candidate path that carries the
//! supplementary checks they ask for.

use super::*;
use archon_workflow::task_set_contract::{
    AcceptanceCheck, AcceptanceCriterion, GapPolicy, JudgeDecision, JudgeVerdict, PrdIdentity,
    TrustedCwd,
};
use archon_workflow::task_skeleton::FrozenTask;

const PRD: &str =
    "# PRD\n\n- REQ-AB-001: raw responses are kept.\n- REQ-AB-002: writes are atomic.\n";

fn entry(id: &str, covers: &[&str]) -> AcceptanceCriterion {
    AcceptanceCriterion {
        id: id.into(),
        criterion: format!("criterion {id}"),
        check: AcceptanceCheck::Command {
            command: "test -f x".into(),
            cwd: TrustedCwd::RepoRoot,
        },
        gap_permitted: false,
        covers: covers.iter().map(|id| id.to_string()).collect(),
        judgment: JudgeVerdict {
            verdict: JudgeDecision::Accepted,
            counterexample: "none".into(),
            reason: "fails when missing".into(),
            host_call_id: "judge".into(),
            sampling: None,
        },
    }
}

fn contract(acceptance: Vec<AcceptanceCriterion>) -> AcceptanceContract {
    AcceptanceContract {
        schema_version: 1,
        prd: PrdIdentity {
            path: "prd.md".into(),
            digest: "d".into(),
        },
        gap_policy: GapPolicy {
            permitted_acceptance_ids: BTreeSet::new(),
            forbidden_phrases: Vec::new(),
            required_fields: Vec::new(),
        },
        acceptance,
        supplementary: Vec::new(),
    }
}

#[test]
fn every_uncovered_requirement_names_the_supplementary_check_it_is_owed() {
    let contract = contract(vec![entry("AC-AB-001", &["REQ-AB-001", "REQ-ZZ-404"])]);
    let findings =
        acceptance_coverage_findings(PRD, Path::new("tasks/acceptance-contract.json"), &contract);
    let texts: Vec<&str> = findings.iter().map(|f| f.text.as_str()).collect();
    assert_eq!(findings.len(), 2, "{texts:#?}");
    // The exact shape the decomposition author parses (acceptanceRepairIds).
    assert!(
        texts[0].starts_with(
            "check 'SUP-REQ-AB-002': PRD requirement REQ-AB-002 is covered by no acceptance check;"
        ) && texts[0].ends_with(": writes are atomic."),
        "{}",
        texts[0]
    );
    assert_eq!(findings[0].subject, "SUP-REQ-AB-002");
    assert!(
        texts[1].starts_with("check 'AC-AB-001': covers names REQ-ZZ-404"),
        "{}",
        texts[1]
    );
    assert!(findings.iter().all(|finding| {
        finding.remediation_scope == archon_workflow::RemediationScope::CandidateArtifact
    }));
}

#[test]
fn new_roles_are_diagnostics_while_covers_remains_validated() {
    let prd = "## Done Definition\n1. Complete.\n\n- NFR-001: Persist evidence.\n- TF-AB-001: Preserve interface.\n\n| ID | Goal |\n|---|---|\n| G-AB-001 | Complete goal. |\n\n| ID | Criterion |\n|---|---|\n| AC-AB-001 | Accepted. |\n";
    let mut contract = contract(vec![entry("AC-AB-001", &["NFR-001"])]);
    let path = Path::new("acceptance-contract.json");
    let findings = acceptance_coverage_findings(prd, path, &contract);
    assert!(findings.is_empty());
    let diagnostics =
        archon_workflow::v2::acceptance_stage::coverage::coverage_diagnostics(prd, &contract);
    assert_eq!(diagnostics.len(), 3);
    for id in ["DONE-1", "G-AB-001", "TF-AB-001"] {
        assert!(diagnostics.iter().any(|text| text.contains(id)));
    }
    contract.acceptance[0]
        .covers
        .extend(["DONE-1", "G-AB-001", "TF-AB-001"].map(String::from));
    assert!(acceptance_coverage_findings(prd, path, &contract).is_empty());
    assert!(
        archon_workflow::v2::acceptance_stage::coverage::coverage_diagnostics(prd, &contract)
            .is_empty()
    );
    contract.acceptance[0].covers.push("NFR-404".into());
    let unknown = acceptance_coverage_findings(prd, path, &contract);
    assert_eq!(unknown.len(), 1);
    assert_eq!(
        unknown[0].deterministic_defect.as_ref().unwrap().code,
        "unknown_covered_requirement"
    );
    assert!(unknown[0].text.contains("coverable PRD ids the host lists"));
}

#[test]
fn a_task_no_check_answers_for_goes_back_to_the_skeleton() {
    let contract = contract(vec![entry("AC-AB-001", &["REQ-AB-001"])]);
    let task = |id: &str, implements: &[&str]| FrozenTask {
        task_id: id.into(),
        file_name: format!("{id}.md"),
        depends_on: Vec::new(),
        blocks: Vec::new(),
        implements: implements.iter().map(|s| s.to_string()).collect(),
        source_sections: Vec::new(),
        deliverable_contracts: Vec::new(),
    };
    let skeleton = TaskSkeleton {
        schema_version: 1,
        acceptance_digest: String::new(),
        tasks: vec![
            task("TASK-A", &["REQ-AB-001"]),
            task("TASK-B", &["REQ-AB-002"]),
        ],
    };
    let findings = skeleton_check_findings(&skeleton, &contract, Path::new("task-skeleton.json"));
    assert_eq!(findings.len(), 1);
    assert!(findings[0].text.starts_with("tasks.TASK-B.implements:"));
    assert_eq!(
        findings[0].remediation_scope,
        archon_workflow::RemediationScope::Skeleton
    );
}

#[test]
fn an_authored_candidate_carries_its_supplementary_checks() {
    let bytes = crate::command::workflow_freeze_candidate::acceptance_candidate(
        br#"{"entries":[{"id":"AC-AB-001","criterion":"","check":{"kind":"command","command":"test -f x","cwd":"project_root"},"gap_permitted":false,"covers":["REQ-AB-001"]}],"supplementary":[{"id":"SUP-REQ-AB-002","criterion":"","check":{"kind":"command","command":"test -f y","cwd":"project_root"},"gap_permitted":false}]}"#,
    )
    .unwrap();
    let contract: AcceptanceContract = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(contract.acceptance[0].covers, ["REQ-AB-001"]);
    assert_eq!(contract.supplementary.len(), 1);
    assert_eq!(contract.supplementary[0].id, "SUP-REQ-AB-002");
}

#[test]
fn preflight_stamps_known_ids_preserves_free_and_acceptance_ids_and_refuses_unknown_requirements() {
    let project = tempfile::tempdir().unwrap();
    let tasks = project.path().join("tasks");
    std::fs::create_dir(&tasks).unwrap();
    let prd = "## Done Definition\n1. Complete.\n- NFR-001: Retain evidence.\n- TF-X-001: Preserve interface.\n\n| ID | Goal |\n|---|---|\n| G-X-001 | Reach goal. |\n\n| ID | Criterion |\n|---|---|\n| AC-X-001 | Accepted. |\n";
    let exact = archon_workflow::obligation_ids::acceptance_criteria(prd);
    let expected = exact.keys().cloned().collect();
    let requirements = archon_workflow::v2::acceptance_stage::coverage::prd_requirement_texts(prd);
    for requirement in requirements.keys().map(String::as_str).chain([
        "UNKNOWN-001",
        "FREE-001",
        "REQ-X-999",
        "AC-X-001",
    ]) {
        for suffix in ["", "-R2"] {
            let mut candidate = contract(vec![entry("AC-X-001", &[])]);
            candidate
                .supplementary
                .push(entry(&format!("SUP-{requirement}{suffix}"), &[]));
            candidate.supplementary[0].check = AcceptanceCheck::Command {
                command: "test -f distinct".into(),
                cwd: TrustedCwd::RepoRoot,
            };
            let result = super::super::preflight::prepare(
                project.path(),
                &tasks,
                &project.path().join("prd.md"),
                "digest",
                prd,
                &exact,
                &expected,
                &serde_json::to_vec(&candidate).unwrap(),
            );
            if let Some(text) = requirements.get(requirement) {
                let prepared = result.unwrap();
                assert_eq!(&prepared.supplementary[0].criterion, text);
                assert_eq!(prepared.supplementary[0].covers, vec![requirement]);
            } else if ["UNKNOWN-001", "FREE-001", "AC-X-001"].contains(&requirement) {
                let prepared = result.unwrap();
                assert_eq!(prepared.supplementary[0], candidate.supplementary[0]);
            } else {
                assert!(format!("{:#}", result.unwrap_err()).contains("does not define"));
            }
        }
    }
}

#[test]
fn coverage_routing_preserves_bullet_and_table_requirements() {
    let prd = "- REQ-X-001: bullet\n\n| ID | Requirement |\n|---|---|\n| REQ-X-002 | table |\n";
    let contract = contract(vec![]);
    let findings = acceptance_coverage_findings(prd, Path::new("contract.json"), &contract);
    assert_eq!(findings.len(), 2);
    assert_eq!(findings[0].subject, "SUP-REQ-X-001");
    assert_eq!(findings[1].subject, "SUP-REQ-X-002");
    let diagnostics =
        archon_workflow::v2::acceptance_stage::coverage::coverage_diagnostics(prd, &contract);
    assert!(diagnostics.is_empty());
}

#[test]
fn coverage_gate_candidate_rejects_duplicate_checks_without_invalidating_frozen_structure() {
    let project = tempfile::tempdir().unwrap();
    let tasks = project.path().join("tasks");
    std::fs::create_dir(&tasks).unwrap();
    let prd = "| ID | Criterion |\n|---|---|\n| AC-X-001 | first |\n| AC-X-002 | second |\n";
    let exact = archon_workflow::obligation_ids::acceptance_criteria(prd);
    let expected = exact.keys().cloned().collect();
    let candidate = contract(vec![entry("AC-X-001", &[]), entry("AC-X-002", &[])]);
    archon_workflow::task_set_contract::validate_acceptance_structure(&candidate, &expected, true)
        .unwrap();
    let error = super::super::preflight::prepare(
        project.path(),
        &tasks,
        &project.path().join("prd.md"),
        "digest",
        prd,
        &exact,
        &expected,
        &serde_json::to_vec(&candidate).unwrap(),
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains(
        "write a check whose command or working directory differs from check 'AC-X-001'"
    ));
}
