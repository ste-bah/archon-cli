//! Map-reduce review contract shapes: the exact-coverage pass, and the
//! gap/duplicate/unbounded and write/non-critic rejections.

use super::*;

#[test]
fn map_reduce_review_contract_passes_for_exact_coverage() {
    let expected = task_set(["TASK-EX-001", "TASK-EX-002"]);
    let details = review_details(
        vec![work_call("implement-task-1")],
        vec![
            review_map_claim(
                "adversarial_findings",
                "adversarial-review-map",
                "TASK-EX-001",
            ),
            review_map_claim(
                "adversarial_findings",
                "adversarial-review-map",
                "TASK-EX-002",
            ),
            review_map_claim(
                "uncovered_requirements",
                "coverage-audit-map",
                "TASK-EX-001",
            ),
            review_map_claim(
                "uncovered_requirements",
                "coverage-audit-map",
                "TASK-EX-002",
            ),
        ],
        vec![
            review_reduce(
                "adversarial_findings",
                "adversarial-review-reduce",
                "adversarial_findings",
                ["adversarial-review-map"],
                [],
            ),
            review_reduce(
                "uncovered_requirements",
                "coverage-audit-reduce",
                "uncovered_requirements",
                ["coverage-audit-map"],
                [],
            ),
        ],
    );
    validate_map_reduce_review_calls(&details, &expected).expect("complete review passes");
}

#[test]
fn map_reduce_review_rejects_gap_duplicate_and_unbounded_reduce() {
    let expected = task_set(["TASK-EX-001", "TASK-EX-002", "TASK-EX-003"]);
    let mut details = review_details(
        vec![work_call("implement-task-1")],
        vec![
            review_map_claim(
                "adversarial_findings",
                "adversarial-review-map",
                "TASK-EX-001",
            ),
            review_map_claim(
                "adversarial_findings",
                "adversarial-review-map",
                "TASK-EX-001",
            ),
            review_map_claim(
                "uncovered_requirements",
                "coverage-audit-map",
                "TASK-EX-001",
            ),
            review_map_claim(
                "uncovered_requirements",
                "coverage-audit-map",
                "TASK-EX-002",
            ),
            review_map_claim(
                "uncovered_requirements",
                "coverage-audit-map",
                "TASK-EX-003",
            ),
        ],
        vec![
            review_reduce(
                "adversarial_findings",
                "adversarial-review-reduce",
                "adversarial_findings",
                ["adversarial-review-map"],
                [],
            ),
            review_reduce(
                "uncovered_requirements",
                "coverage-audit-reduce",
                "uncovered_requirements",
                ["coverage-audit-map"],
                [],
            ),
        ],
    );
    details.review_reduce_edges[0].max_input_bytes = None;
    let error =
        validate_map_reduce_review_calls(&details, &expected).expect_err("bad review rejected");
    assert!(error.contains("TASK-EX-002"), "{error}");
    assert!(error.contains("TASK-EX-003"), "{error}");
    assert!(error.contains("more than once"), "{error}");
    assert!(error.contains("maxInputBytes"), "{error}");
}

#[test]
fn map_reduce_review_rejects_write_and_non_critic_reviews() {
    let expected = task_set(["TASK-EX-001"]);
    let mut details = review_details(
        vec![work_call("implement-task-1")],
        vec![
            review_map_claim(
                "adversarial_findings",
                "adversarial-review-map",
                "TASK-EX-001",
            ),
            review_map_claim(
                "uncovered_requirements",
                "coverage-audit-map",
                "TASK-EX-001",
            ),
        ],
        vec![
            review_reduce(
                "adversarial_findings",
                "adversarial-review-reduce",
                "adversarial_findings",
                ["adversarial-review-map"],
                [],
            ),
            review_reduce(
                "uncovered_requirements",
                "coverage-audit-reduce",
                "uncovered_requirements",
                ["coverage-audit-map"],
                [],
            ),
        ],
    );
    details.calls[1].write_mode = Some(WorkflowV2WriteMode::Worktree);
    details.calls[2].options.role = Some("coder".to_string());
    let error =
        validate_map_reduce_review_calls(&details, &expected).expect_err("unsafe review rejected");
    assert!(error.contains("read-only"), "{error}");
    assert!(error.contains("tier 'critic'"), "{error}");
}

#[test]
fn a_remediation_call_may_repeat_the_task_its_verifier_failed() {
    // The brief demands one initial write call per task AND a bounded
    // remediation loop re-running a write agent for that same task. Counting
    // every repeat as a duplicate claim made both instructions unsatisfiable:
    // a live attempt was rejected for writing exactly what it was told to.
    let claims = vec![
        ("TASK-A".to_string(), "agents-1".to_string()),
        ("TASK-B".to_string(), "agents-2".to_string()),
        ("TASK-A".to_string(), "remediate-task-a-2-5".to_string()),
        ("TASK-B".to_string(), "remediate-task-b-2-9".to_string()),
    ];
    assert!(
        super::v3_author_checks_a::cross_ownership_defects(&claims).is_empty(),
        "{:?}",
        super::v3_author_checks_a::cross_ownership_defects(&claims)
    );
}

#[test]
fn a_second_owner_claiming_other_tasks_too_is_still_a_defect() {
    let claims = vec![
        ("TASK-A".to_string(), "agents-1".to_string()),
        ("TASK-A".to_string(), "agents-2".to_string()),
        ("TASK-C".to_string(), "agents-2".to_string()),
    ];
    let defects = super::v3_author_checks_a::cross_ownership_defects(&claims);
    assert_eq!(defects.len(), 1, "{defects:?}");
    assert!(defects[0].contains("TASK-A"), "{defects:?}");
}
