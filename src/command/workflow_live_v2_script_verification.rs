use super::*;

pub(super) fn focused_verification_accepted_task_ids(
    call: &WorkflowV2HostCall,
    result: &WorkflowV2Result,
) -> std::collections::BTreeSet<String> {
    if call.options.item_kind.as_deref() != Some("focused_verification") {
        return Default::default();
    }
    result
        .data
        .get("outcomes")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(accepted_focused_completion_evidence)
        .collect()
}

pub(super) fn accepted_focused_completion_evidence(outcome: &serde_json::Value) -> Vec<String> {
    outcome
        .get("completion_evidence")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| focused_completion_evidence_valid(item))
        .filter_map(|item| item.get("task_id").and_then(serde_json::Value::as_str))
        .map(str::to_string)
        .collect()
}

pub(super) fn focused_completion_evidence_valid(item: &serde_json::Value) -> bool {
    let accepted = matches!(
        item.get("status").and_then(serde_json::Value::as_str),
        Some("accepted" | "noop")
    );
    let versioned = item.get("source_fingerprint").and_then(serde_json::Value::as_str)
        == Some("focused-verification-evidence-v2");
    let kind = item.get("evidence_kind").and_then(serde_json::Value::as_str)
        == Some("focused_verification");
    accepted && versioned && kind && focused_completion_has_refs(item)
}

pub(super) fn focused_completion_has_refs(item: &serde_json::Value) -> bool {
    ["artifact_paths", "command_refs", "evidence_refs"]
        .iter()
        .any(|key| {
            item.get(*key)
                .and_then(serde_json::Value::as_array)
                .is_some_and(|values| !values.is_empty())
        })
}
