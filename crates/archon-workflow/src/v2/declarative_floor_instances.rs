use super::declarative_floor_consistency::internal_consistency;
use super::declarative_floor_facts::{DeclarativeFloorFacts, FileState, InstanceBinding};
use super::declarative_floor_values::{artifact_format, json_file};
use crate::task_universe::WorkflowV2DeliverableContract;

pub(super) fn instances(
    contract: &WorkflowV2DeliverableContract,
    facts: &DeclarativeFloorFacts,
    findings: &mut Vec<String>,
) {
    let format = artifact_format(contract, &contract.artifact_path);
    if !matches!(format.as_str(), "json" | "text") {
        findings.push(format!("unsupported declared artifact_format: {format}"));
        return;
    }
    if let Some(InstanceBinding::SourceBound { records }) = &facts.instance_binding {
        if let Some(source) = source_path(contract) {
            let value = json_file(
                facts.files.get(source),
                "declared instance source",
                findings,
            );
            if let Some(value) = value {
                let path = records_field(contract);
                let records = super::declarative_floor::field(&value, path);
                if records.is_some_and(|v| !v.is_object() && !v.is_array()) {
                    findings.push(format!(
                        "instance source records field is not a collection: {}",
                        path.unwrap_or_default()
                    ));
                }
            }
        }
        for (key, reference) in records {
            if reference.is_none() {
                findings.push(format!(
                    "source entry {key} has no artifact reference field {}",
                    contract
                        .instance_artifact_field
                        .as_deref()
                        .unwrap_or_default()
                ));
            }
        }
    }
    if facts.instances.len() < contract.min_instances {
        findings.push(format!(
            "declared deliverable requires >= {} instance(s), found {} for {}",
            contract.min_instances,
            facts.instances.len(),
            contract.artifact_path
        ));
    }
    for instance in &facts.instances {
        let label = format!("declared deliverable instance {}", instance.fact.shown);
        if artifact_format(contract, &instance.reference) == "json" {
            if let Some(value) = json_file(Some(&instance.fact), &label, findings) {
                internal_consistency(contract, &value, &label, findings);
            }
        } else if !matches!(&instance.fact.state, FileState::Bytes(bytes) if !bytes.is_empty()) {
            match &instance.fact.state {
                FileState::Unreadable(error) => {
                    findings.push(format!("{label} cannot be read: {error}"))
                }
                _ => findings.push(format!(
                    "declared deliverable instance missing or empty: {}",
                    instance.fact.shown
                )),
            }
        }
    }
}

pub(super) fn source_path(contract: &WorkflowV2DeliverableContract) -> Option<&str> {
    contract
        .instance_source_path
        .as_deref()
        .or(contract.registry_path.as_deref())
}
pub(super) fn records_field(contract: &WorkflowV2DeliverableContract) -> Option<&str> {
    contract
        .instance_source_records_field
        .as_deref()
        .or(contract.registry_records_field.as_deref())
}
