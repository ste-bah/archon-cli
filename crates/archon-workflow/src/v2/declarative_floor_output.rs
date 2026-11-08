use super::declarative_floor::{FloorVerdict, field};
use super::declarative_floor_facts::DeclarativeFloorFacts;
use super::declarative_floor_values::artifact_format;
use crate::task_universe::WorkflowV2DeliverableContract;
use serde_json::{Value, json};

/// The shared machine-readable verdict for callers that render an evaluation.
pub fn verdict_value(
    contract: &WorkflowV2DeliverableContract,
    facts: &DeclarativeFloorFacts,
    verdict: FloorVerdict,
) -> Value {
    if let FloorVerdict::Failed { findings } = verdict {
        return json!({"status":"failed", "failure_count":findings.len(), "failures":findings});
    }
    let artifact = facts
        .files
        .get(&contract.artifact_path)
        .map(|file| file.shown.as_str())
        .unwrap_or(&contract.artifact_path);
    if contract.artifact_path.contains('<') && contract.artifact_path.contains('>') {
        json!({"status":"declared_deliverable_instances_present", "pattern":contract.artifact_path,"instance_count":facts.instances.len()})
    } else if artifact_format(contract, &contract.artifact_path) == "text" {
        json!({"status":"declared_text_deliverable_present", "artifact":artifact,"bytes":facts.artifact_byte_len})
    } else if !contract.required_universe {
        json!({"status":"declared_deliverable_present", "artifact":artifact})
    } else {
        let cells = facts
            .artifact_json
            .as_ref()
            .and_then(|value| field(value, contract.cells_field.as_deref()))
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or_default();
        let required = facts
            .artifact_json
            .as_ref()
            .map(|artifact| {
                super::declarative_floor::required_identities(contract, artifact, &mut Vec::new())
                    .len()
            })
            .unwrap_or_default();
        let series = facts
            .artifact_json
            .as_ref()
            .and_then(|artifact| field(artifact, contract.cells_field.as_deref()))
            .and_then(Value::as_array)
            .map(|cells| {
                if contract.series_value_fields.is_empty() {
                    0
                } else {
                    cells.len()
                }
            })
            .unwrap_or_default();
        json!({"status":"substantive_deliverable_verified", "required_cells":required,"verified_cells":cells,"series_checked":series})
    }
}
