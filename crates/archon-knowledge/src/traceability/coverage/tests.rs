use super::*;
use crate::traceability::requirements::{Severity, extract_requirements};

fn reqs() -> Vec<Requirement> {
    extract_requirements(concat!(
        "- REQ-DL-001: one.\n",
        "- REQ-DL-002: two.\n",
        "- REQ-DL-003: three.\n",
    ))
}

fn binding(task_id: &str, implements: &[&str]) -> TaskBinding {
    TaskBinding {
        task_id: task_id.to_string(),
        source_path: format!("tests/{task_id}.md"),
        implements: implements.iter().map(|s| s.to_string()).collect(),
        ..TaskBinding::default()
    }
}

#[test]
fn exact_coverage_both_ways() {
    let report = check_coverage(
        &reqs(),
        &[
            binding("TASK-A", &["REQ-DL-001", "REQ-DL-002"]),
            binding("TASK-B", &["REQ-DL-003"]),
        ],
    );
    assert!(report.is_exact());
    assert_eq!(report.requirements_total, 3);
    assert_eq!(report.citations_total, 3);
    assert_eq!(report.claimed_by["REQ-DL-001"], ["TASK-A"]);
    assert!(report.multiply_claimed.is_empty());
}

#[test]
fn an_unclaimed_requirement_is_a_decomposition_gap_not_an_invention() {
    let report = check_coverage(&reqs(), &[binding("TASK-A", &["REQ-DL-001"])]);
    assert_eq!(report.unclaimed, ["REQ-DL-002", "REQ-DL-003"]);
    assert!(!report.is_exact());
    // Nothing was fabricated to close the gap.
    assert_eq!(report.claimed_by.len(), 1);
}

#[test]
fn a_citation_the_prd_does_not_define_is_a_phantom() {
    let report = check_coverage(
        &reqs(),
        &[binding(
            "TASK-A",
            &["REQ-DL-001", "REQ-DL-002", "REQ-DL-999"],
        )],
    );
    assert_eq!(report.phantom.len(), 1);
    assert_eq!(report.phantom[0].cited_id, "REQ-DL-999");
    assert_eq!(report.phantom[0].task_id, "TASK-A");
    assert_eq!(report.phantom[0].source_path, "tests/TASK-A.md");
    assert!(!report.is_exact());
    // A phantom does not claim the real requirement it resembles.
    assert_eq!(report.unclaimed, ["REQ-DL-003"]);
}

#[test]
fn multiply_claimed_requirements_are_named_not_faulted() {
    let report = check_coverage(
        &reqs(),
        &[
            binding("TASK-A", &["REQ-DL-001", "REQ-DL-002"]),
            binding("TASK-B", &["REQ-DL-001", "REQ-DL-003"]),
        ],
    );
    assert!(report.is_exact());
    assert_eq!(report.multiply_claimed, ["REQ-DL-001"]);
    assert_eq!(report.claimed_by["REQ-DL-001"], ["TASK-A", "TASK-B"]);
}

#[test]
fn severity_survives_extraction_for_downstream_scoping() {
    let requirements = extract_requirements("- REQ-DL-131: Unknown status must fail closed.\n");
    assert_eq!(requirements[0].severity, Severity::Error);
    let report = check_coverage(&requirements, &[]);
    assert_eq!(report.unclaimed, ["REQ-DL-131"]);
}
