use serde_json::{Map, Value};

use archon_workflow::generated_lifecycle_support as support;

pub(super) fn reroute_unplanned_raw_task_identity(
    mut triage: Value,
    plan_items: &[Value],
) -> Value {
    let Some(data) = triage_data_mut(&mut triage) else {
        return triage;
    };
    let failures = support::array(data.get("implementation_failures"));
    let mut retained = Vec::new();
    let mut retries = support::array(data.get("retry_items"));
    let mut corrections = support::array(data.get("overreach_corrections"));
    for failure in failures {
        let Some(plan) = matching_plan(&failure, plan_items) else {
            retained.push(failure);
            continue;
        };
        let raw_task_identity_overreach = is_unplanned_raw_task_identity_check(&failure, plan);
        let host_manifest_overreach = is_host_manifest_schema_overreach(&failure);
        if !raw_task_identity_overreach && !host_manifest_overreach {
            retained.push(failure);
            continue;
        }
        if host_manifest_overreach {
            retries.push(corrected_host_manifest_retry(&failure, plan));
            corrections.push(host_manifest_correction_record(&failure));
        } else {
            retries.push(corrected_retry(&failure, plan));
            corrections.push(correction_record(&failure));
        }
    }
    data.insert(
        "implementation_failures".to_string(),
        Value::Array(retained),
    );
    data.insert("retry_items".to_string(), Value::Array(retries));
    data.insert(
        "overreach_corrections".to_string(),
        Value::Array(corrections),
    );
    triage
}

fn triage_data_mut(triage: &mut Value) -> Option<&mut Map<String, Value>> {
    if triage.get("data").is_some() {
        return triage.get_mut("data")?.as_object_mut();
    }
    if triage.pointer("/result/data").is_some() {
        return triage.pointer_mut("/result/data")?.as_object_mut();
    }
    triage.as_object_mut()
}

fn matching_plan<'a>(failure: &Value, plans: &'a [Value]) -> Option<&'a Value> {
    let source_id = failure
        .get("source_item_id")
        .or_else(|| failure.get("item_id"))
        .and_then(Value::as_str)?;
    plans.iter().find(|plan| {
        ["item_id", "id", "source_item_id"]
            .into_iter()
            .filter_map(|key| plan.get(key).and_then(Value::as_str))
            .any(|id| ids_match(source_id, id))
    })
}

fn ids_match(left: &str, right: &str) -> bool {
    left == right || left.ends_with(&format!("-{right}")) || right.ends_with(&format!("-{left}"))
}

fn is_unplanned_raw_task_identity_check(failure: &Value, plan: &Value) -> bool {
    let failure_text = searchable_text(failure);
    references_raw_provider_artifact(&failure_text)
        && demands_task_identity(&failure_text)
        && !plan_requires_task_identity(plan)
}

fn references_raw_provider_artifact(text: &str) -> bool {
    [
        "raw/request.json",
        "raw/provider-notes.md",
        "raw/headers.redacted.json",
    ]
    .iter()
    .any(|path| text.contains(path))
}

fn demands_task_identity(text: &str) -> bool {
    text.contains("task-id")
        || text.contains("task_id")
        || text.contains("canonical task")
        || text.contains("task identity")
        || contains_canonical_task_reference(text)
            && (text.contains("-specific") || text.contains("identif"))
}

fn contains_canonical_task_reference(text: &str) -> bool {
    text.split(|character: char| !character.is_ascii_alphanumeric() && character != '-')
        .any(is_canonical_task_reference)
}

fn is_canonical_task_reference(candidate: &str) -> bool {
    let mut parts = candidate.split('-');
    matches!(parts.next(), Some(prefix) if prefix.eq_ignore_ascii_case("task"))
        && matches!(parts.next(), Some(namespace) if !namespace.is_empty() && namespace.chars().all(|character| character.is_ascii_alphanumeric()))
        && matches!(parts.next(), Some(sequence) if sequence.len() == 3 && sequence.chars().all(|character| character.is_ascii_digit()))
        && parts.next().is_none()
}

fn plan_requires_task_identity(plan: &Value) -> bool {
    let fields = serde_json::json!({
        "focused_verification": plan.get("focused_verification"),
        "expected_evidence": plan.get("expected_evidence"),
        "artifact_requirements": plan.get("artifact_requirements"),
    });
    demands_task_identity(&searchable_text(&fields))
}

fn is_host_manifest_schema_overreach(failure: &Value) -> bool {
    let text = searchable_text(failure);
    references_host_patch_manifest(&text)
        && [
            "provider_env_proof",
            "source_item_id",
            "canonical_task_ids",
            "normalized_path",
            "write_coordination_scope",
            "evidence field",
        ]
        .iter()
        .any(|field| text.contains(field))
}

fn references_host_patch_manifest(text: &str) -> bool {
    text.contains("write-coordination")
        || text.contains("write coordination manifest")
        || text.contains("patch_manifest.v1")
        || text.contains("manifest-level proof")
}

fn searchable_text(value: &Value) -> String {
    serde_json::to_string(value)
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn corrected_retry(failure: &Value, plan: &Value) -> Value {
    let source_id = failure
        .get("source_item_id")
        .and_then(Value::as_str)
        .unwrap_or("verification-outcome");
    serde_json::json!({
        "item_id": format!("retry-overreach-{source_id}"),
        "source_item_id": source_id,
        "canonical_task_ids": support::strings_of(failure.get("canonical_task_ids")),
        "source_residual_gap_ids": support::strings_of(failure.get("source_residual_gap_ids")),
        "failed_predicate": failure.get("failed_predicate"),
        "classification": "retryable_verification_shape_issue",
        "verification_failure_class": "artifact_contract_overreach",
        "focused_verification": plan.get("focused_verification"),
        "expected_evidence": plan.get("expected_evidence"),
        "artifact_requirements": plan.get("artifact_requirements"),
        "recommended_retry": "Rerun only the original TASK artifact contract; do not require canonical task IDs on raw provider payloads."
    })
}

fn corrected_host_manifest_retry(failure: &Value, plan: &Value) -> Value {
    let source_id = failure
        .get("source_item_id")
        .and_then(Value::as_str)
        .unwrap_or("verification-outcome");
    serde_json::json!({
        "item_id": format!("retry-host-manifest-overreach-{source_id}"),
        "source_item_id": source_id,
        "canonical_task_ids": support::strings_of(failure.get("canonical_task_ids")),
        "source_residual_gap_ids": support::strings_of(failure.get("source_residual_gap_ids")),
        "failed_predicate": failure.get("failed_predicate"),
        "classification": "retryable_verification_shape_issue",
        "verification_failure_class": "host_manifest_schema_overreach",
        "focused_verification": [
            "Validate only the archon.workflow.patch_manifest.v1 host schema fields: schema, run_id, stage_id, item_id, baseline_commit, patch_path, declared_target_files, changed_files, created_files, deleted_files, pre_hashes, post_hashes, verify_command, agent_artifact_path, and status.",
            "Resolve provider_env_proof from the run-scoped workflow input or run evidence, never from the host patch manifest."
        ],
        "expected_evidence": plan.get("expected_evidence"),
        "artifact_requirements": plan.get("artifact_requirements"),
        "recommended_retry": "Rerun the original verification using the authoritative host manifest schema. Do not require provider_env_proof, source_item_id, canonical_task_ids, normalized_path, write_coordination_scope, or provider evidence fields inside the patch manifest."
    })
}

fn correction_record(failure: &Value) -> Value {
    serde_json::json!({
        "source_item_id": failure.get("source_item_id"),
        "source_residual_gap_ids": failure.get("source_residual_gap_ids"),
        "classification": "artifact_contract_overreach",
        "reason": "raw provider payload task identity was not required by the source verification plan"
    })
}

fn host_manifest_correction_record(failure: &Value) -> Value {
    serde_json::json!({
        "source_item_id": failure.get("source_item_id"),
        "source_residual_gap_ids": failure.get("source_residual_gap_ids"),
        "classification": "host_manifest_schema_overreach",
        "reason": "verifier required fields outside the host-owned archon.workflow.patch_manifest.v1 schema"
    })
}

#[cfg(test)]
#[path = "workflow_live_v2_lifecycle_verify_overreach_tests.rs"]
mod tests;
