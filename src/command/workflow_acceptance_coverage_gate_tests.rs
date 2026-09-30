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
fn a_task_no_check_answers_for_goes_back_to_the_skeleton() {
    let contract = contract(vec![entry("AC-AB-001", &["REQ-AB-001"])]);
    let task = |id: &str, implements: &[&str]| FrozenTask {
        task_id: id.into(),
        file_name: format!("{id}.md"),
        depends_on: Vec::new(),
        blocks: Vec::new(),
        implements: implements.iter().map(|s| s.to_string()).collect(),
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
