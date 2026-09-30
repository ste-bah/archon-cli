use super::*;

fn input() -> Value {
    serde_json::json!({
        "item": {"canonical_task_ids": ["TDL-020"]},
        "task_universe": {
            "schema_version": "workflow-v2-task-universe-v1",
            "source_roots": [],
            "tasks": [
                {"canonical_task_id": "TASK-A-010", "acceptance_criteria": ["other"]},
                {"canonical_task_id": "TASK-A-020", "aliases": ["TDL-020"],
                 "acceptance_criteria": ["first thing holds", "second thing holds"]},
            ]
        }
    })
}

fn accepted(data: Value) -> WorkflowV2Result {
    WorkflowV2Result {
        status: WorkflowV2Status::Accepted,
        summary: "done".into(),
        data,
        ..WorkflowV2Result::default()
    }
}

#[test]
fn the_criteria_come_from_the_claimed_task_in_the_universe() {
    let criteria = claimed_criteria(&input());
    assert_eq!(criteria.len(), 2, "{criteria:?}");
    assert_eq!(criteria[0].task_id, "TASK-A-020", "the alias resolves");
    assert_eq!(criteria[1].index, 2);
    assert_eq!(criteria[1].text, "second thing holds");
}

#[test]
fn every_criterion_met_with_evidence_stays_accepted() {
    let mut result = accepted(serde_json::json!({
        "criterion_results": [
            {"task_id": "TASK-A-020", "criterion_index": 1, "status": "met", "evidence": "ran x: ok"},
            {"criterion": "second thing holds", "status": "MET", "evidence": ["test y passed"]},
        ]
    }));
    enforce(&input(), &mut result);
    assert_eq!(result.status, WorkflowV2Status::Accepted);
    assert_eq!(result.data[CRITERION_CHECK_KEY]["met"], 2);
    assert!(result.residual_gaps.is_empty());
}

#[test]
fn a_missing_unmet_or_unevidenced_criterion_demotes_and_is_named() {
    for entries in [
        serde_json::json!([{"criterion_index": 1, "status": "met", "evidence": "ok"}]),
        serde_json::json!([
            {"criterion_index": 1, "status": "met", "evidence": "ok"},
            {"criterion_index": 2, "status": "unmet", "evidence": "fails"},
        ]),
        serde_json::json!([
            {"criterion_index": 1, "status": "met", "evidence": "ok"},
            {"criterion_index": 2, "status": "met", "evidence": "  "},
        ]),
    ] {
        let mut result = accepted(serde_json::json!({ "criterion_results": entries }));
        enforce(&input(), &mut result);
        assert_eq!(result.status, WorkflowV2Status::NeedsReview, "{entries}");
        let gap = &result.residual_gaps[0];
        assert_eq!(gap.id, "unmet_acceptance_criteria_TASK-A-020");
        assert!(
            gap.description.contains("#2 \"second thing holds\""),
            "{}",
            gap.description
        );
        assert!(!gap.description.contains("#1 "), "{}", gap.description);
        assert_eq!(
            result.data[CRITERION_CHECK_KEY]["unmet"][0]["criterion_index"],
            2
        );
    }
}

#[test]
fn no_criterion_results_at_all_demotes() {
    let mut result = accepted(serde_json::json!({}));
    enforce(&input(), &mut result);
    assert_eq!(result.status, WorkflowV2Status::NeedsReview);
    assert_eq!(result.data[CRITERION_CHECK_KEY]["required"], 2);
}

#[test]
fn an_honest_non_acceptance_or_a_task_without_criteria_is_left_alone() {
    let mut blocked = accepted(serde_json::json!({}));
    blocked.status = WorkflowV2Status::Blocked;
    enforce(&input(), &mut blocked);
    assert_eq!(blocked.status, WorkflowV2Status::Blocked);
    assert!(blocked.data.get(CRITERION_CHECK_KEY).is_none());

    let mut none = accepted(serde_json::json!({}));
    let unclaimed = serde_json::json!({"item": {"canonical_task_ids": ["NOPE"]}});
    enforce(&unclaimed, &mut none);
    assert_eq!(none.status, WorkflowV2Status::Accepted);
}

#[test]
fn an_entry_for_another_task_does_not_count() {
    let mut result = accepted(serde_json::json!({
        "criterion_results": [
            {"task_id": "TASK-A-010", "criterion_index": 1, "status": "met", "evidence": "ok"},
            {"task_id": "TASK-A-010", "criterion_index": 2, "status": "met", "evidence": "ok"},
        ]
    }));
    enforce(&input(), &mut result);
    assert_eq!(result.status, WorkflowV2Status::NeedsReview);
}

#[test]
fn the_prompt_lists_every_criterion_by_index() {
    let text = prompt_section(&input());
    assert!(text.contains("data.criterion_results"), "{text}");
    assert!(text.contains("TASK-A-020 #1: first thing holds"), "{text}");
    assert!(text.contains("TASK-A-020 #2: second thing holds"), "{text}");
    assert!(prompt_section(&serde_json::json!({})).is_empty());
}
