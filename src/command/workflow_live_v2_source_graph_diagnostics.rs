use super::*;

pub(super) fn source_data_contract_issue(
    values: &[serde_json::Value],
    universe: &TaskUniverse,
    task_universe: &WorkflowV2TaskUniverse,
    wave_kind: DynamicSourceKind,
) -> String {
    for (index, value) in values.iter().enumerate() {
        let normalized = normalize_generated_item_value(value, Some(task_universe)).value;
        if item_id(&normalized).is_none() {
            return source_field_issue(index, "item_id", "is missing or empty");
        }
        let tasks = raw_task_refs(&normalized);
        if tasks.is_empty() {
            return source_field_issue(index, "canonical_task_ids", "is missing or empty");
        }
        if tasks.iter().all(|task| universe.resolve(task).is_none()) {
            return source_field_issue(index, "canonical_task_ids", "contains no canonical task");
        }
        if let Some(field) = missing_kind_field(&normalized, wave_kind) {
            return source_field_issue(index, field, "is missing or empty");
        }
    }
    "source_data failed canonical graph construction or target expansion".to_string()
}

fn missing_kind_field(value: &serde_json::Value, kind: DynamicSourceKind) -> Option<&'static str> {
    match kind {
        DynamicSourceKind::Remediation => missing_remediation_field(value),
        DynamicSourceKind::ReviewRemediation => missing_review_remediation_field(value),
        DynamicSourceKind::FocusedVerification | DynamicSourceKind::ReviewVerification => {
            missing_verification_field(value)
        }
        DynamicSourceKind::NoopProof => missing_noop_field(value),
        DynamicSourceKind::Implementation => None,
    }
}

fn missing_remediation_field(value: &serde_json::Value) -> Option<&'static str> {
    for field in [
        "source_item_id",
        "failure_status",
        "failure_evidence",
        "required_fix",
        "verification_requirements",
        "focused_verification",
    ] {
        if !value_present(value.get(field)) {
            return Some(field);
        }
    }
    missing_present_key(value, &["target_files", "dependency_ids"])
        .or_else(|| missing_evidence_field(value))
}

fn missing_review_remediation_field(value: &serde_json::Value) -> Option<&'static str> {
    for field in [
        "source_item_id",
        "failure_status",
        "failure_evidence",
        "required_fix",
    ] {
        if !value_present(value.get(field)) {
            return Some(field);
        }
    }
    missing_present_key(value, &["target_files", "dependency_ids"])
        .or_else(|| missing_verification_field(value))
}

fn missing_verification_field(value: &serde_json::Value) -> Option<&'static str> {
    if !value_present(value.get("focused_verification")) {
        return Some("focused_verification");
    }
    missing_evidence_field(value)
}

fn missing_noop_field(value: &serde_json::Value) -> Option<&'static str> {
    ["noop_proof", "noop_proof_refs", "acceptance_criteria"]
        .into_iter()
        .find(|field| !value_present(value.get(*field)))
}

fn missing_evidence_field(value: &serde_json::Value) -> Option<&'static str> {
    (!verification_evidence_fields_present(value))
        .then_some("expected_evidence/artifact_requirements")
}

fn missing_present_key(value: &serde_json::Value, fields: &[&'static str]) -> Option<&'static str> {
    fields
        .iter()
        .copied()
        .find(|field| value.get(*field).is_none())
}

fn source_field_issue(index: usize, field: &str, reason: &str) -> String {
    format!("source_data[{index}].{field} {reason}")
}
