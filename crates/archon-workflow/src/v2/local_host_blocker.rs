// Deterministic blocker context for non-accepted final reports.
//
// When the lifecycle stops on a blocked/needs-review final report, the
// lifecycle-provided call inputs carry the concrete failure (unresolved
// outcomes, an unschedulable follow-up remediation inventory, prior wave
// evidence). The host lifts a compact digest of that context into the
// stored report so the blocker survives into `results/` instead of being
// reduced to aggregate missing-task counts.

use super::*;

use crate::v2::outcome_envelope::outcomes_of;

const DIGEST_TEXT_LIMIT: usize = 400;

pub(super) fn final_report_blocker_context(
    execution: &WorkflowV2CallExecution,
) -> Option<serde_json::Value> {
    let inputs = execution.input.get("inputs")?;
    let mut blocker = serde_json::Map::new();
    blocker.insert(
        "call_id".to_string(),
        serde_json::Value::String(execution.call.id.clone()),
    );
    insert_outcome_digests(inputs, &mut blocker);
    if let Some(inventory) = inputs.get("unscheduledFollowupInventory")
        && !inventory.is_null()
    {
        blocker.insert(
            "unscheduled_followup_inventory".to_string(),
            inventory_digest(inventory),
        );
    }
    let evidence = evidence_digest(inputs.get("implementationEvidence"));
    if !evidence.is_empty() {
        blocker.insert(
            "implementation_evidence_digest".to_string(),
            serde_json::Value::Array(evidence),
        );
    }
    (blocker.len() > 1).then_some(serde_json::Value::Object(blocker))
}

fn insert_outcome_digests(
    inputs: &serde_json::Value,
    blocker: &mut serde_json::Map<String, serde_json::Value>,
) {
    for (source_key, digest_key) in [
        ("unresolvedAfterRemediation", "unresolved_outcomes"),
        ("unresolved", "unresolved_outcomes"),
        ("failedImplementationOutcomes", "failed_outcomes"),
        ("reviewVerification", "failed_outcomes"),
        ("verification", "failed_outcomes"),
    ] {
        if blocker.contains_key(digest_key) {
            continue;
        }
        let digest = outcome_digest(inputs.get(source_key));
        if !digest.is_empty() {
            blocker.insert(digest_key.to_string(), serde_json::Value::Array(digest));
        }
    }
}

fn outcome_digest(value: Option<&serde_json::Value>) -> Vec<serde_json::Value> {
    match value {
        Some(serde_json::Value::Array(outcomes)) => {
            outcomes.iter().map(outcome_digest_entry).collect()
        }
        Some(value @ serde_json::Value::Object(_)) => vec![outcome_digest_entry(value)],
        _ => Vec::new(),
    }
}

fn outcome_digest_entry(outcome: &serde_json::Value) -> serde_json::Value {
    let result = outcome.get("result").unwrap_or(outcome);
    serde_json::json!({
        "item_id": outcome.get("item_id").or_else(|| outcome.get("id")),
        "status": result.get("status").or_else(|| outcome.get("status")),
        "summary": digest_text(result.get("summary").or_else(|| outcome.get("summary"))),
    })
}

fn inventory_digest(inventory: &serde_json::Value) -> serde_json::Value {
    let items: Vec<serde_json::Value> = inventory
        .get("items")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|item| {
                    serde_json::json!({
                        "item_id": item.get("item_id").or_else(|| item.get("id")),
                        "canonical_task_ids": item.get("canonical_task_ids"),
                        "target_files": item.get("target_files"),
                        "required_fix": digest_text(item.get("required_fix")),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    serde_json::json!({
        "items": items,
        "unresolved_issues": inventory.get("unresolved_issues"),
    })
}

fn evidence_digest(value: Option<&serde_json::Value>) -> Vec<serde_json::Value> {
    value
        .and_then(serde_json::Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .map(|entry| {
                    let outcomes = entry
                        .get("result")
                        .map(|result| {
                            outcomes_of(result)
                                .iter()
                                .map(outcome_digest_entry)
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    serde_json::json!({
                        "kind": entry.get("kind"),
                        "implementationWaveIndex": entry.get("implementationWaveIndex"),
                        "dependencyIteration": entry.get("dependencyIteration"),
                        "remediationAttempt": entry.get("remediationAttempt"),
                        "outcomes": outcomes,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn digest_text(value: Option<&serde_json::Value>) -> serde_json::Value {
    match value.and_then(serde_json::Value::as_str) {
        Some(text) if text.chars().count() > DIGEST_TEXT_LIMIT => serde_json::Value::String(
            text.chars()
                .take(DIGEST_TEXT_LIMIT)
                .chain("…".chars())
                .collect(),
        ),
        Some(text) => serde_json::Value::String(text.to_string()),
        None => serde_json::Value::Null,
    }
}

#[cfg(test)]
mod blocker_tests {
    use crate::{WorkflowV2CallExecution, WorkflowV2HostCall, WorkflowV2HostMethod};

    fn execution(input: serde_json::Value) -> WorkflowV2CallExecution {
        WorkflowV2CallExecution {
            call: WorkflowV2HostCall {
                id: "blocked-remediation-unresolved-1".to_string(),
                method: WorkflowV2HostMethod::FinalReport,
                write_mode: None,
                options: Default::default(),
            },
            input,
            depends_on: Vec::new(),
        }
    }

    #[test]
    fn blocker_context_preserves_unresolved_and_followup() {
        let input = serde_json::json!({
            "status": "needs_review",
            "inputs": {
                "unresolvedAfterRemediation": [{
                    "item_id": "REM-A-001",
                    "result": { "status": "needs_review", "summary": "schema repair rejected" },
                }],
                "unscheduledFollowupInventory": {
                    "items": [{
                        "item_id": "REM-A-001-1",
                        "canonical_task_ids": ["TASK-A-001"],
                        "target_files": ["src/lib.rs"],
                        "required_fix": "restore evidence fields",
                    }],
                    "unresolved_issues": [{
                        "field": "target_files",
                        "message": "target file looks like instruction text",
                    }],
                },
            },
        });
        let blocker = super::final_report_blocker_context(&execution(input))
            .expect("blocker context present");
        assert_eq!(
            blocker["call_id"],
            serde_json::json!("blocked-remediation-unresolved-1")
        );
        assert_eq!(
            blocker["unresolved_outcomes"][0]["item_id"],
            serde_json::json!("REM-A-001")
        );
        assert_eq!(
            blocker["unresolved_outcomes"][0]["summary"],
            serde_json::json!("schema repair rejected")
        );
        let followup = &blocker["unscheduled_followup_inventory"];
        assert_eq!(
            followup["items"][0]["item_id"],
            serde_json::json!("REM-A-001-1")
        );
        assert_eq!(
            followup["unresolved_issues"][0]["message"],
            serde_json::json!("target file looks like instruction text")
        );
    }

    #[test]
    fn blocker_context_absent_without_lifecycle_inputs() {
        assert!(super::final_report_blocker_context(&execution(serde_json::json!({}))).is_none());
    }
}
