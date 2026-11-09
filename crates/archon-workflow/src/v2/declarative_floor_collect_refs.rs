use super::declarative_floor::{field, registry_key};
use super::declarative_floor_instances::{records_field, source_path};
use super::declarative_floor_records::reference;
use crate::task_universe::WorkflowV2DeliverableContract;
use serde_json::Value;
use std::collections::BTreeSet;

pub fn referenced_files(
    contract: &WorkflowV2DeliverableContract,
    artifact: Option<&Value>,
    registry: Option<&Value>,
) -> Vec<String> {
    let mut paths = BTreeSet::new();
    if contract.artifact_path.contains('<')
        && let Some(path) = source_path(contract)
    {
        paths.insert(path.to_owned());
    }
    if let Some(cells) = artifact
        .and_then(|value| field(value, contract.cells_field.as_deref()))
        .and_then(Value::as_array)
    {
        let records =
            registry.and_then(|value| field(value, contract.registry_records_field.as_deref()));
        for cell in cells {
            let record = records.and_then(|records| records.get(registry_key(contract, cell)));
            if let Some(record) = record {
                for path in [
                    &contract.validation_path_field,
                    &contract.payload_path_field,
                    &contract.request_path_field,
                    &contract.response_path_field,
                ]
                .into_iter()
                .flatten()
                {
                    if let Some(reference) = reference(record, path) {
                        paths.insert(reference.to_owned());
                    }
                }
            }
        }
    }
    paths.into_iter().collect()
}

pub(super) fn instance_records(
    contract: &WorkflowV2DeliverableContract,
    source: &Value,
) -> Vec<(String, Option<String>)> {
    let Some(path) = contract.instance_artifact_field.as_deref() else {
        return Vec::new();
    };
    let entry = |key, record: &Value| (key, reference(record, path).map(str::to_owned));
    match field(source, records_field(contract)) {
        Some(Value::Object(records)) => {
            let mut entries: Vec<_> = records.iter().map(|(k, v)| entry(k.clone(), v)).collect();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            entries
        }
        Some(Value::Array(records)) => records
            .iter()
            .enumerate()
            .map(|(k, v)| entry(k.to_string(), v))
            .collect(),
        _ => Vec::new(),
    }
}
