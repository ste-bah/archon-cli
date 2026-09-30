//! Which PRD requirements the frozen checks exercise (H4, A12).

use std::collections::BTreeSet;

use super::*;
use crate::task_set_contract::{
    AcceptanceCheck, GapPolicy, JudgeDecision, JudgeVerdict, PrdIdentity, TrustedCwd,
};
use crate::task_universe::WorkflowV2TaskUniverseTask;

const PRD: &str = "# PRD\n\n## Requirements\n\n- REQ-AB-001: the store keeps raw responses.\n- REQ-AB-002: normalized rows are replayable.\n- REQ-AB-003: writes are atomic.\n\n## Acceptance\n\n| ID | Criterion |\n|---|---|\n| AC-AB-001 | status shows the root |\n| AC-AB-002 | ingest writes the artifact set |\n";

fn entry(id: &str, covers: &[&str]) -> AcceptanceCriterion {
    AcceptanceCriterion {
        id: id.into(),
        criterion: format!("criterion {id}"),
        check: AcceptanceCheck::Command {
            command: "test -f x".into(),
            cwd: TrustedCwd::RepoRoot,
        },
        gap_permitted: false,
        judgment: JudgeVerdict {
            verdict: JudgeDecision::Accepted,
            counterexample: "none".into(),
            reason: "fails when missing".into(),
            host_call_id: "judge".into(),
            sampling: None,
        },
        covers: covers.iter().map(|id| id.to_string()).collect(),
    }
}

fn contract(
    acceptance: Vec<AcceptanceCriterion>,
    supplementary: Vec<AcceptanceCriterion>,
) -> AcceptanceContract {
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
        supplementary,
    }
}

#[test]
fn requirement_ids_come_from_the_prd_alone() {
    let ids = prd_requirement_ids(PRD);
    assert_eq!(
        ids.into_iter().collect::<Vec<_>>(),
        vec!["REQ-AB-001", "REQ-AB-002", "REQ-AB-003"]
    );
    assert_eq!(
        prd_requirement_texts(PRD)["REQ-AB-003"],
        "writes are atomic."
    );
}

#[test]
fn uncovered_requirements_are_the_prd_ids_no_check_covers() {
    let ids = prd_requirement_ids(PRD);
    let legacy = contract(
        vec![entry("AC-AB-001", &[]), entry("AC-AB-002", &[])],
        Vec::new(),
    );
    assert_eq!(
        uncovered_requirements(&ids, &legacy),
        vec!["REQ-AB-001", "REQ-AB-002", "REQ-AB-003"]
    );
    let partial = contract(
        vec![
            entry("AC-AB-001", &["REQ-AB-001"]),
            entry("AC-AB-002", &["REQ-AB-002"]),
        ],
        Vec::new(),
    );
    assert_eq!(uncovered_requirements(&ids, &partial), vec!["REQ-AB-003"]);
    assert_eq!(supplementary_id("REQ-AB-003"), "SUP-REQ-AB-003");
    assert_eq!(
        supplementary_requirement("SUP-REQ-AB-003"),
        Some("REQ-AB-003")
    );
    assert_eq!(supplementary_requirement("SUP-OTHER"), None);
    let full = contract(
        partial.acceptance.clone(),
        vec![entry("SUP-REQ-AB-003", &["REQ-AB-003"])],
    );
    assert!(uncovered_requirements(&ids, &full).is_empty());
}

#[test]
fn drift_and_unknown_covers_are_named() {
    let ids = prd_requirement_ids(PRD);
    let drifted = contract(
        vec![entry("AC-AB-001", &["REQ-AB-001", "REQ-ZZ-999"])],
        Vec::new(),
    );
    let acceptance = crate::obligation_ids::acceptance_ids(PRD);
    assert_eq!(
        missing_acceptance_ids(&acceptance, &drifted),
        vec!["AC-AB-002"]
    );
    assert_eq!(
        unknown_covers(&ids, &drifted),
        vec![("AC-AB-001".to_string(), "REQ-ZZ-999".to_string())]
    );
}

#[test]
fn a_task_is_answered_for_through_a_check_id_or_a_covered_requirement() {
    let covered = contract(
        vec![entry("AC-AB-001", &["REQ-AB-001"])],
        vec![entry("SUP-REQ-AB-003", &["REQ-AB-003"])],
    );
    let (a, b, c, d) = (
        vec!["AC-AB-001".to_string()],
        vec!["REQ-AB-003".to_string()],
        vec!["REQ-AB-002".to_string()],
        Vec::new(),
    );
    let tasks = [
        ("TASK-A", a.as_slice()),
        ("TASK-B", b.as_slice()),
        ("TASK-C", c.as_slice()),
        ("TASK-D", d.as_slice()),
    ];
    assert_eq!(
        tasks_without_checks(tasks, &covered),
        vec!["TASK-C", "TASK-D"]
    );
    let task = |id: &str, implements: &[&str]| WorkflowV2TaskUniverseTask {
        canonical_task_id: id.into(),
        source_path: format!("tasks/{id}.md"),
        implements: implements.iter().map(|s| s.to_string()).collect(),
        ..WorkflowV2TaskUniverseTask::default()
    };
    let universe = WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: vec!["tasks".into()],
        tasks: vec![
            task("TASK-B", &["REQ-AB-003"]),
            task("TASK-A", &["AC-AB-001"]),
        ],
    };
    assert_eq!(
        implementing_tasks(Some(&universe), "SUP-REQ-AB-003", &["REQ-AB-003".into()]),
        vec!["TASK-B"]
    );
    assert_eq!(
        implementing_tasks(Some(&universe), "AC-AB-001", &[]),
        vec!["TASK-A"]
    );
}
