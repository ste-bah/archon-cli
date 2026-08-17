use super::verification_plan_criteria_gaps;

fn universe() -> serde_json::Value {
    serde_json::json!({
        "tasks": [
            {
                "canonical_task_id": "TASK-A-010",
                "acceptance_criteria": [
                    "Every persisted dataset reports production_eligible true",
                    "The registry contains 30 cells",
                ],
            },
            {
                "canonical_task_id": "TASK-B-020",
                "acceptance_criteria": ["An unrelated task criterion"],
            },
        ]
    })
}

fn plan(items: serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "items": items })
}

/// The live loss. A plan whose entire promise is that the code compiles is
/// "ready" by shape, runs a full wave, and accepts every branch without ever
/// touching what the task was written to deliver.
#[test]
fn a_compile_only_plan_leaves_every_criterion_uncovered() {
    let plan = plan(serde_json::json!([{
        "item_id": "verification-a-010-cargo-check",
        "canonical_task_ids": ["TASK-A-010"],
        "focused_verification": ["modules compile: cargo check"],
        "expected_evidence": ["cargo check -p some-crate exits 0"],
    }]));

    let gaps = verification_plan_criteria_gaps(&universe(), &["TASK-A-010".to_string()], &plan);

    assert_eq!(gaps.len(), 2, "{gaps:#?}");
    assert_eq!(gaps[0]["canonical_task_id"], "TASK-A-010");
}

#[test]
fn an_item_claiming_its_criteria_outright_closes_them() {
    let plan = plan(serde_json::json!([{
        "canonical_task_ids": ["TASK-A-010"],
        "covered_acceptance_criteria": [
            "Every persisted dataset reports production_eligible true",
            "The registry contains 30 cells",
        ],
    }]));

    assert!(
        verification_plan_criteria_gaps(&universe(), &["TASK-A-010".to_string()], &plan).is_empty()
    );
}

/// Coverage may arrive as objects rather than bare strings; a plan that did the
/// work must not fail the gate on shape.
#[test]
fn coverage_entries_may_be_objects() {
    let plan = plan(serde_json::json!([{
        "canonical_task_ids": ["TASK-A-010"],
        "covered_acceptance_criteria": [
            {"criterion": "Every persisted dataset reports production_eligible true"},
            {"text": "The registry contains 30 cells"},
        ],
    }]));

    assert!(
        verification_plan_criteria_gaps(&universe(), &["TASK-A-010".to_string()], &plan).is_empty()
    );
}

/// Evidence that quotes the criterion whole is a real promise to check it.
#[test]
fn a_criterion_quoted_in_expected_evidence_counts_as_covered() {
    let plan = plan(serde_json::json!([{
        "canonical_task_ids": ["TASK-A-010"],
        "expected_evidence": [
            "registry.json shows every persisted dataset reports production_eligible true, read after ingest",
            "the registry contains 30 cells after the run",
        ],
    }]));

    assert!(
        verification_plan_criteria_gaps(&universe(), &["TASK-A-010".to_string()], &plan).is_empty()
    );
}

/// Containment runs one way. Were it symmetric, a short generic evidence line
/// would be "contained in" every long criterion and close the whole plan.
#[test]
fn short_generic_evidence_cannot_swallow_a_long_criterion() {
    let plan = plan(serde_json::json!([{
        "canonical_task_ids": ["TASK-A-010"],
        "expected_evidence": ["registry"],
        "focused_verification": ["dataset"],
    }]));

    let gaps = verification_plan_criteria_gaps(&universe(), &["TASK-A-010".to_string()], &plan);

    assert_eq!(gaps.len(), 2, "{gaps:#?}");
}

#[test]
fn only_uncovered_criteria_are_reported() {
    let plan = plan(serde_json::json!([{
        "canonical_task_ids": ["TASK-A-010"],
        "covered_acceptance_criteria": ["The registry contains 30 cells"],
    }]));

    let gaps = verification_plan_criteria_gaps(&universe(), &["TASK-A-010".to_string()], &plan);

    assert_eq!(gaps.len(), 1);
    assert_eq!(
        gaps[0]["criterion"],
        "Every persisted dataset reports production_eligible true"
    );
}

/// One item runs the command, another inspects the artifact. Between them the
/// task is covered, and neither is failed for not covering it alone.
#[test]
fn claims_pool_across_items_of_the_same_task() {
    let plan = plan(serde_json::json!([
        {
            "canonical_task_ids": ["TASK-A-010"],
            "covered_acceptance_criteria": ["The registry contains 30 cells"],
        },
        {
            "canonical_task_ids": ["TASK-A-010"],
            "covered_acceptance_criteria": [
                "Every persisted dataset reports production_eligible true"
            ],
        },
    ]));

    assert!(
        verification_plan_criteria_gaps(&universe(), &["TASK-A-010".to_string()], &plan).is_empty()
    );
}

/// A wave verifies the work it implemented. Tasks it never touched are not its
/// to prove, and reporting them would block every wave but the last.
#[test]
fn tasks_outside_the_wave_are_not_gated() {
    let plan = plan(serde_json::json!([{
        "canonical_task_ids": ["TASK-A-010"],
        "covered_acceptance_criteria": [
            "Every persisted dataset reports production_eligible true",
            "The registry contains 30 cells",
        ],
    }]));

    assert!(
        verification_plan_criteria_gaps(&universe(), &["TASK-A-010".to_string()], &plan).is_empty(),
        "TASK-B-020 is not in this wave"
    );
}

/// Criterion text is copied task file -> prompt -> model reply and picks up
/// case and whitespace on the way. That is not a missing check.
#[test]
fn case_and_whitespace_differences_are_not_gaps() {
    let plan = plan(serde_json::json!([{
        "canonical_task_ids": ["task-a-010"],
        "covered_acceptance_criteria": [
            "EVERY   persisted dataset\n reports production_eligible TRUE",
            "the registry contains 30 cells",
        ],
    }]));

    assert!(
        verification_plan_criteria_gaps(&universe(), &["TASK-A-010".to_string()], &plan).is_empty()
    );
}

/// Absent criteria are the universe parser's problem and are gated upstream.
/// Reporting them here would double-report and block on someone else's defect.
#[test]
fn a_candidate_without_criteria_contributes_no_gaps() {
    let universe = serde_json::json!({
        "tasks": [{"canonical_task_id": "TASK-C-030", "acceptance_criteria": []}]
    });
    let plan = plan(serde_json::json!([{"canonical_task_ids": ["TASK-C-030"]}]));

    assert!(
        verification_plan_criteria_gaps(&universe, &["TASK-C-030".to_string()], &plan).is_empty()
    );
}

/// An empty candidate list means there is nothing this wave must prove; the
/// gate must not then demand coverage of the entire universe.
#[test]
fn no_candidates_means_no_gate() {
    assert!(
        verification_plan_criteria_gaps(&universe(), &[], &plan(serde_json::json!([]))).is_empty()
    );
}
