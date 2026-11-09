use super::declarative_floor::{field, normalized};
use crate::task_universe::WorkflowV2DeliverableContract;
use serde_json::Value;

pub(super) fn internal_consistency(
    contract: &WorkflowV2DeliverableContract,
    value: &Value,
    label: &str,
    findings: &mut Vec<String>,
) {
    if !value.is_object() {
        return;
    }
    let (Some(status), Some(checks), Some(check_status)) = (
        contract.validation_status_field.as_deref(),
        contract.validation_checks_field.as_deref(),
        contract.validation_check_status_field.as_deref(),
    ) else {
        return;
    };
    let empty = Vec::new();
    let checks = match field(value, Some(checks)) {
        None => &empty,
        Some(Value::Array(checks)) => checks,
        _ => {
            findings.push(format!("{label} checks field is not an array"));
            return;
        }
    };
    let failed = checks
        .iter()
        .filter(|check| {
            let actual = field(check, Some(check_status))
                .map(normalized)
                .unwrap_or_default();
            contract
                .validation_failed_values
                .iter()
                .any(|value| value.trim().to_lowercase() == actual)
        })
        .count();
    let overall = field(value, Some(status))
        .map(normalized)
        .unwrap_or_default();
    if failed > 0
        && contract
            .validation_passed_values
            .iter()
            .any(|value| value.trim().to_lowercase() == overall)
    {
        findings.push(format!(
            "{label} is internally inconsistent: overall={overall} with {failed} failed check(s)"
        ));
    }
}

pub(super) fn declarations(contract: &WorkflowV2DeliverableContract, findings: &mut Vec<String>) {
    let kind = contract.data_kind.as_deref().unwrap_or_default().trim();
    let payload = contract.payload_path_field.is_some()
        || !contract.required_fields.is_empty()
        || !contract.non_constant_fields.is_empty()
        || !contract.series_value_fields.is_empty();
    if !kind.is_empty() && kind != "record_series" {
        findings.push(format!("unsupported declared data_kind: {kind}"));
    }
    if payload && kind != "record_series" {
        findings.push("payload substance declarations require data_kind=record_series".into());
    }
    if kind == "record_series" {
        for (name, missing) in [
            ("payload_path_field", contract.payload_path_field.is_none()),
            ("required_fields", contract.required_fields.is_empty()),
            (
                "non_constant_fields",
                contract.non_constant_fields.is_empty(),
            ),
            (
                "series_value_fields",
                contract.series_value_fields.is_empty(),
            ),
        ] {
            if missing {
                findings.push(format!("record_series contract has no {name}"));
            }
        }
    }
}
