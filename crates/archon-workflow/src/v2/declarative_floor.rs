//! Pure commandless deliverable-floor evaluation.
//!
//! Filesystem and JSON collection happen in the host. This module receives
//! typed facts only and never renders or executes command text. The existing
//! verifier and the run-end observer share its eligibility and deterministic
//! predicates; command-bearing or unsupported advanced checks are explicit
//! deferrals rather than host fallbacks.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::task_universe::WorkflowV2DeliverableContract;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeclarativeFloorFacts {
    pub artifact_present: bool,
    pub artifact_byte_len: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_json: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry_json: Option<Value>,
    #[serde(default)]
    pub instance_count: usize,
    /// The roots the collector tried, in order (Issue-22). Empty when the facts
    /// were not collected from disk; a single entry reports as it always did.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub searched_roots: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum DeclarativeFloorEvaluation {
    Passed,
    Failed { findings: Vec<String> },
    Deferred { reason: String },
}

pub fn evaluate_declarative_floor(
    contract: &WorkflowV2DeliverableContract,
    facts: &DeclarativeFloorFacts,
) -> DeclarativeFloorEvaluation {
    if let Some(reason) = declarative_floor_deferral_reason(contract) {
        return DeclarativeFloorEvaluation::Deferred { reason };
    }
    let mut findings = Vec::new();
    if !facts.artifact_present || facts.artifact_byte_len == 0 {
        findings.push(format!(
            "declared deliverable missing or empty: {}{}",
            contract.artifact_path,
            searched_roots_suffix(&contract.artifact_path, &facts.searched_roots)
        ));
        return DeclarativeFloorEvaluation::Failed { findings };
    }
    let format = contract
        .artifact_format
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase)
        .unwrap_or_else(|| {
            if contract.artifact_path.ends_with(".json")
                || contract.artifact_path.ends_with(".jsonl")
            {
                "json".to_string()
            } else {
                "text".to_string()
            }
        });
    if format == "text" {
        return finish(findings);
    }
    if format != "json" {
        findings.push(format!("unsupported declared artifact_format: {format}"));
        return finish(findings);
    }
    let Some(artifact) = facts.artifact_json.as_ref() else {
        findings.push("declared deliverable is not valid JSON".to_string());
        return finish(findings);
    };
    if contract.registry_path.is_some() && facts.registry_json.is_none() {
        findings.push("declared registry is missing, empty, or not valid JSON".to_string());
    }
    if !contract.required_universe {
        return finish(findings);
    }
    evaluate_required_universe(
        contract,
        artifact,
        facts.registry_json.as_ref(),
        &mut findings,
    );
    finish(findings)
}

/// ` (looked under a, b)` when a relative path was sought under several roots
/// and found under none, so the finding says where the host looked. One root
/// keeps the historical text; an absolute path consulted no root.
fn searched_roots_suffix(artifact_path: &str, searched_roots: &[String]) -> String {
    let path = std::path::Path::new(artifact_path);
    if searched_roots.len() < 2 || path.is_absolute() || path.has_root() {
        return String::new();
    }
    format!(" (looked under {})", searched_roots.join(", "))
}

fn evaluate_required_universe(
    contract: &WorkflowV2DeliverableContract,
    artifact: &Value,
    registry: Option<&Value>,
    findings: &mut Vec<String>,
) {
    let cells = field(artifact, contract.cells_field.as_deref()).and_then(Value::as_array);
    let Some(cells) = cells else {
        findings.push(format!(
            "declared cells field is not an array: {}",
            contract.cells_field.as_deref().unwrap_or("")
        ));
        return;
    };
    if contract.cell_identity_fields.is_empty() {
        findings.push("required-universe contract has no cell_identity_fields".to_string());
        return;
    }
    let required = required_identities(contract, artifact, findings);
    let mut indexed = BTreeMap::<Vec<String>, &Value>::new();
    for cell in cells {
        let identity = identity(cell, &contract.cell_identity_fields);
        if indexed.insert(identity.clone(), cell).is_some() {
            findings.push(format!("duplicate cell identity: {identity:?}"));
        }
    }
    if !required.is_empty() {
        for missing in required.difference(&indexed.keys().cloned().collect()) {
            findings.push(format!("missing required cell: {missing:?}"));
        }
        for extra in indexed
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>()
            .difference(&required)
        {
            findings.push(format!("extra undeclared cell: {extra:?}"));
        }
    }
    let gaps = contract
        .gaps_field
        .as_deref()
        .and_then(|path| field(artifact, Some(path)))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if !gaps.is_empty() {
        findings.push(format!(
            "declared deliverable contains {} gap record(s)",
            gaps.len()
        ));
    }
    let records = registry_records(contract, registry, findings);
    for (cell_identity, cell) in indexed {
        let label = cell_identity.join(":");
        evaluate_cell_fields(contract, cell, &label, findings);
        evaluate_registry_fields(contract, cell, records, &label, findings);
    }
}

fn required_identities(
    contract: &WorkflowV2DeliverableContract,
    artifact: &Value,
    findings: &mut Vec<String>,
) -> BTreeSet<Vec<String>> {
    let mut axes = Vec::new();
    for path in &contract.universe_fields {
        let values = field(artifact, Some(path)).and_then(Value::as_array);
        let Some(values) = values.filter(|values| !values.is_empty()) else {
            findings.push("declared required universe has an empty or non-array axis".to_string());
            return BTreeSet::new();
        };
        axes.push(values.iter().map(identity_value).collect::<Vec<_>>());
    }
    cartesian(&axes)
}

fn cartesian(axes: &[Vec<String>]) -> BTreeSet<Vec<String>> {
    let mut rows = BTreeSet::from([Vec::new()]);
    for axis in axes {
        let mut next = BTreeSet::new();
        for row in &rows {
            for value in axis {
                let mut extended = row.clone();
                extended.push(value.clone());
                next.insert(extended);
            }
        }
        rows = next;
    }
    rows
}

fn evaluate_cell_fields(
    contract: &WorkflowV2DeliverableContract,
    cell: &Value,
    label: &str,
    findings: &mut Vec<String>,
) {
    for path in &contract.required_true_fields {
        if field(cell, Some(path)) != Some(&Value::Bool(true)) {
            findings.push(format!("{label} required true field failed: {path}"));
        }
    }
    for path in &contract.required_nonempty_fields {
        if field(cell, Some(path)).is_none_or(empty) {
            findings.push(format!("{label} required non-empty field failed: {path}"));
        }
    }
    for path in &contract.positive_count_fields {
        if numeric(field(cell, Some(path))).is_none_or(|value| value <= 0) {
            findings.push(format!("{label} positive count field failed: {path}"));
        }
    }
    for (path, minimum) in &contract.minimum_count_fields {
        match numeric(field(cell, Some(path))) {
            Some(value) if value >= *minimum as i128 => {}
            Some(value) => findings.push(format!(
                "{label} count below declared minimum: {path}={value} < {minimum}"
            )),
            None => findings.push(format!(
                "{label} minimum count field is not numeric: {path}"
            )),
        }
    }
}

fn evaluate_registry_fields(
    contract: &WorkflowV2DeliverableContract,
    cell: &Value,
    records: Option<&serde_json::Map<String, Value>>,
    label: &str,
    findings: &mut Vec<String>,
) {
    if contract.registry_path.is_none() {
        return;
    }
    let key = contract
        .registry_key_fields
        .iter()
        .map(|path| identity_value(field(cell, Some(path)).unwrap_or(&Value::Null)))
        .collect::<Vec<_>>()
        .join(":");
    let record = records.and_then(|records| records.get(&key));
    let Some(record) = record else {
        findings.push(format!(
            "{label} has no declared registry record for key {key}"
        ));
        return;
    };
    for path in &contract.registry_required_true_fields {
        if field(record, Some(path)) != Some(&Value::Bool(true)) {
            findings.push(format!(
                "{label} registry required true field failed: {path}"
            ));
        }
    }
    if let Some(path) = &contract.registry_status_field {
        let actual = field(record, Some(path))
            .map(normalized)
            .unwrap_or_default();
        let allowed = contract
            .registry_allowed_statuses
            .iter()
            .map(|value| value.trim().to_ascii_lowercase())
            .collect::<BTreeSet<_>>();
        if !allowed.contains(&actual) {
            findings.push(format!("{label} registry status is not allowed"));
        }
    }
    if let Some(path) = &contract.registry_count_field {
        let value = numeric(field(record, Some(path)));
        if value.is_none_or(|value| value <= 0) {
            findings.push(format!("{label} registry count is not positive: {path}"));
        } else if contract.registry_minimum_count > 0
            && value.unwrap_or_default() < contract.registry_minimum_count as i128
        {
            findings.push(format!(
                "{label} registry count below declared minimum: {path}={} < {}",
                value.unwrap_or_default(),
                contract.registry_minimum_count
            ));
        }
    }
    for (cell_path, record_path) in &contract.registry_identity_fields {
        if field(cell, Some(cell_path)).map(normalized)
            != field(record, Some(record_path)).map(normalized)
        {
            findings.push(format!(
                "{label} registry identity mismatch: {cell_path}!={record_path}"
            ));
        }
    }
}

fn registry_records<'a>(
    contract: &WorkflowV2DeliverableContract,
    registry: Option<&'a Value>,
    findings: &mut Vec<String>,
) -> Option<&'a serde_json::Map<String, Value>> {
    let registry = registry?;
    let records = contract
        .registry_records_field
        .as_deref()
        .and_then(|path| field(registry, Some(path)))
        .and_then(Value::as_object);
    if records.is_none() {
        findings.push("declared registry records field is not an object".to_string());
    }
    records
}

/// Why this contract must stay on the existing command-capable verifier path.
///
/// The observer never executes deferred text. The implementation verifier uses
/// this same boundary and retains its established generated verifier for these
/// cases, so extracting the pure floor does not narrow existing enforcement.
pub fn declarative_floor_deferral_reason(
    contract: &WorkflowV2DeliverableContract,
) -> Option<String> {
    if contract
        .typed_verifier_command
        .as_deref()
        .is_some_and(|command| !command.trim().is_empty())
    {
        return Some(
            "typed_verifier_command requires isolated command execution, deferred in R2a"
                .to_string(),
        );
    }
    if contract.artifact_path.contains('<')
        || contract.instance_source_path.is_some()
        || contract.instance_source_records_field.is_some()
        || contract.instance_artifact_field.is_some()
    {
        return Some(
            "parameterized deliverable collection is deferred in the R2a run-end observer"
                .to_string(),
        );
    }
    if advanced_checks_declared(contract) {
        return Some(
            "advanced payload, temporal, validation, or cross-series predicates are deferred in the R2a run-end observer"
                .to_string(),
        );
    }
    None
}

fn advanced_checks_declared(contract: &WorkflowV2DeliverableContract) -> bool {
    contract.data_kind.is_some()
        || contract.payload_path_field.is_some()
        || contract.observed_time_field.is_some()
        || contract.validation_path_field.is_some()
        || contract.request_path_field.is_some()
        || contract.response_path_field.is_some()
        || !contract.series_value_fields.is_empty()
        || !contract.required_fields.is_empty()
        || !contract.non_constant_fields.is_empty()
}

fn identity(value: &Value, fields: &[String]) -> Vec<String> {
    fields
        .iter()
        .map(|path| {
            field(value, Some(path))
                .map(identity_value)
                .unwrap_or_default()
        })
        .collect()
}

fn field<'a>(value: &'a Value, path: Option<&str>) -> Option<&'a Value> {
    let mut current = value;
    for part in path?.split('.') {
        current = current.as_object()?.get(part)?;
    }
    Some(current)
}

fn identity_value(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Null => String::new(),
        value => value.to_string(),
    }
}

fn normalized(value: &Value) -> String {
    identity_value(value).trim().to_ascii_lowercase()
}

fn numeric(value: Option<&Value>) -> Option<i128> {
    match value? {
        Value::Number(number) => number
            .as_i64()
            .map(i128::from)
            .or_else(|| number.as_u64().map(i128::from)),
        Value::String(value) => value.trim().parse().ok(),
        _ => None,
    }
}

fn empty(value: &Value) -> bool {
    matches!(value, Value::Null)
        || value.as_str().is_some_and(str::is_empty)
        || value.as_array().is_some_and(Vec::is_empty)
        || value.as_object().is_some_and(serde_json::Map::is_empty)
}

fn finish(findings: Vec<String>) -> DeclarativeFloorEvaluation {
    if findings.is_empty() {
        DeclarativeFloorEvaluation::Passed
    } else {
        DeclarativeFloorEvaluation::Failed { findings }
    }
}
