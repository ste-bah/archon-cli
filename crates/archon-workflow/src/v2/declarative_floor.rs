//! Pure commandless deliverable-floor evaluation.
//!
//! Filesystem and JSON collection happen in the host. This module receives
//! typed facts only and never renders or executes command text. Callers run
//! authored commands separately after evaluating these predicates.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::task_universe::WorkflowV2DeliverableContract;

use super::declarative_floor_consistency::{declarations, internal_consistency};
pub use super::declarative_floor_facts::DeclarativeFloorFacts;
use super::declarative_floor_records::evaluate_records;
use super::declarative_floor_series::Series;
use super::declarative_floor_values::{
    artifact_format, identity_equal, identity_key, identity_text, scalar_text,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum FloorVerdict {
    Passed,
    Failed { findings: Vec<String> },
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
    if let Some(defect) = super::deliverable_contract::contract_defect(
        &serde_json::to_value(contract).expect("contract value"),
    ) {
        return DeclarativeFloorEvaluation::Failed {
            findings: vec![defect],
        };
    }
    if let Some(reason) = declarative_floor_deferral_reason(contract) {
        return DeclarativeFloorEvaluation::Deferred { reason };
    }
    match evaluate_floor_predicates(contract, facts) {
        FloorVerdict::Passed => DeclarativeFloorEvaluation::Passed,
        FloorVerdict::Failed { findings } => DeclarativeFloorEvaluation::Failed { findings },
    }
}

pub fn evaluate_floor_predicates(
    contract: &WorkflowV2DeliverableContract,
    facts: &DeclarativeFloorFacts,
) -> FloorVerdict {
    let mut findings = facts.collection_findings.clone();
    let raw = serde_json::to_value(contract).expect("contract value");
    if let Some(defect) = super::deliverable_contract::contract_defect(&raw) {
        return FloorVerdict::Failed {
            findings: vec![defect],
        };
    }
    if contract.artifact_path.contains('<') && contract.artifact_path.contains('>') {
        super::declarative_floor_instances::instances(contract, facts, &mut findings);
        return finish(findings);
    }
    if let Some(fact) = facts.files.get(&contract.artifact_path)
        && let super::declarative_floor_facts::FileState::Unreadable(error) = &fact.state
    {
        return FloorVerdict::Failed {
            findings: vec![format!(
                "declared deliverable cannot be read: {}: {error}",
                fact.shown
            )],
        };
    }
    if !facts.artifact_present || facts.artifact_byte_len == 0 {
        let shown = facts
            .files
            .get(&contract.artifact_path)
            .map(|fact| fact.shown.clone())
            .unwrap_or_else(|| {
                format!(
                    "{}{}",
                    contract.artifact_path,
                    searched_roots_suffix(&contract.artifact_path, &facts.searched_roots)
                )
            });
        findings.push(format!("declared deliverable missing or empty: {shown}"));
        return finish(findings);
    }
    let format = artifact_format(contract, &contract.artifact_path);
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
    if artifact.is_null() {
        return FloorVerdict::Failed {
            findings: vec!["declared deliverable contains a null value".into()],
        };
    }
    internal_consistency(contract, artifact, "declared deliverable", &mut findings);
    declarations(contract, &mut findings);
    if contract.registry_path.is_some() && facts.registry_json.is_none() {
        if let Some(fact) = contract
            .registry_path
            .as_ref()
            .and_then(|path| facts.files.get(path))
        {
            super::declarative_floor_values::json_file(
                Some(fact),
                "declared registry",
                &mut findings,
            );
        } else {
            findings.push("declared registry is missing, empty, or not valid JSON".to_string());
        }
    }
    if facts.registry_json.as_ref().is_some_and(Value::is_null) {
        findings.push("declared registry contains a null value".into());
    }
    if !contract.required_universe {
        return finish(findings);
    }
    evaluate_required_universe(contract, artifact, facts, &mut findings);
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
    facts: &DeclarativeFloorFacts,
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
            findings.push(format!(
                "duplicate cell identity: {}",
                identity_text(&identity)
            ));
        }
    }
    if !required.is_empty() {
        for missing in required.difference(&indexed.keys().cloned().collect()) {
            findings.push(format!("missing required cell: {}", identity_text(missing)));
        }
        for extra in indexed
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>()
            .difference(&required)
        {
            findings.push(format!("extra undeclared cell: {}", identity_text(extra)));
        }
    }
    let gaps = contract
        .gaps_field
        .as_deref()
        .and_then(|path| field(artifact, Some(path)))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let gap_count = contract
        .gaps_field
        .as_deref()
        .and_then(|path| field(artifact, Some(path)))
        .map(|value| match value {
            Value::Array(values) => values.len(),
            Value::Object(values) => values.len(),
            Value::String(value) => value.chars().count(),
            Value::Null | Value::Bool(false) => 0,
            _ => 1,
        })
        .unwrap_or_default();
    if gap_count > 0 {
        findings.push(format!(
            "declared deliverable contains {gap_count} gap record(s)"
        ));
    }
    let records = registry_records(contract, facts.registry_json.as_ref(), findings);
    let mut series = Series::default();
    for (cell_identity, cell) in indexed {
        let label = cell_identity
            .iter()
            .map(|key| {
                serde_json::from_str::<Value>(key)
                    .map(|value| scalar_text(&value))
                    .unwrap_or_else(|_| key.clone())
            })
            .collect::<Vec<_>>()
            .join(":");
        for path in &contract.required_true_fields {
            if field(cell, Some(path)) != Some(&Value::Bool(true)) {
                findings.push(format!("{label} required true field failed: {path}"));
            }
        }
        if gaps
            .iter()
            .any(|gap| identity(gap, &contract.cell_identity_fields) == cell_identity)
        {
            continue;
        }
        evaluate_cell_fields(contract, cell, &label, findings);
        if contract
            .registry_key_fields
            .iter()
            .any(|path| field(cell, Some(path)).is_none_or(Value::is_null))
        {
            findings.push(format!(
                "{label} registry key contains a null or missing value"
            ));
            continue;
        }
        let registry_available = facts
            .registry_json
            .as_ref()
            .is_some_and(|value| !value.is_null());
        evaluate_registry_fields(
            contract,
            cell,
            records,
            registry_available,
            &label,
            findings,
        );
        let key = registry_key(contract, cell);
        let record = records.and_then(|records| records.get(&key));
        if registry_available && record.is_none_or(|record| !record.is_object()) {
            continue;
        }
        let rows = evaluate_records(
            contract,
            facts,
            cell,
            record.unwrap_or(&Value::Null),
            &label,
            findings,
        );
        series.cell(contract, &rows, &label, findings);
    }
    series.finish(contract, findings);
}

pub(super) fn required_identities(
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
        axes.push(values.iter().map(identity_key).collect::<Vec<_>>());
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
    registry_available: bool,
    label: &str,
    findings: &mut Vec<String>,
) {
    let key = registry_key(contract, cell);
    let record = records
        .and_then(|records| records.get(&key))
        .filter(|record| record.is_object());
    if registry_available && record.is_none() {
        findings.push(format!(
            "{label} has no declared registry record for key {key}"
        ));
        return;
    }
    let record = record.unwrap_or(&Value::Null);
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
        if !identity_equal(
            field(cell, Some(cell_path)),
            field(record, Some(record_path)),
        ) {
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
    if registry.is_null() {
        return None;
    }
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

/// Only the authored command needs a separate execution boundary.
pub fn declarative_floor_deferral_reason(
    contract: &WorkflowV2DeliverableContract,
) -> Option<String> {
    contract.typed_verifier_command.as_deref().filter(|text| !text.trim().is_empty()).map(|_| "typed_verifier_command runs a command; the floor's predicates are evaluated natively and the command runs separately".to_owned())
}

pub(super) fn registry_key(contract: &WorkflowV2DeliverableContract, cell: &Value) -> String {
    contract
        .registry_key_fields
        .iter()
        .map(|path| scalar_text(field(cell, Some(path)).unwrap_or(&Value::Null)))
        .collect::<Vec<_>>()
        .join(":")
}

fn identity(value: &Value, fields: &[String]) -> Vec<String> {
    fields
        .iter()
        .map(|path| {
            field(value, Some(path))
                .map(identity_key)
                .unwrap_or_else(|| "null".into())
        })
        .collect()
}

pub(super) fn field<'a>(value: &'a Value, path: Option<&str>) -> Option<&'a Value> {
    let mut current = value;
    for part in path?.split('.') {
        current = current.as_object()?.get(part)?;
    }
    Some(current)
}

pub(super) fn normalized(value: &Value) -> String {
    if value.is_null() {
        String::new()
    } else {
        scalar_text(value).trim().to_lowercase()
    }
}

fn numeric(value: Option<&Value>) -> Option<i128> {
    super::declarative_floor_values::integer(value)
}

fn empty(value: &Value) -> bool {
    matches!(value, Value::Null)
        || value.as_str().is_some_and(str::is_empty)
        || value.as_array().is_some_and(Vec::is_empty)
        || value.as_object().is_some_and(serde_json::Map::is_empty)
}

fn finish(findings: Vec<String>) -> FloorVerdict {
    if findings.is_empty() {
        FloorVerdict::Passed
    } else {
        FloorVerdict::Failed { findings }
    }
}
