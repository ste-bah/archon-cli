//! The task schema and hint share the serialized FrozenTask shape.
use crate::task_skeleton::{ConsumedArtifact, FrozenDependency, FrozenTask};
use crate::task_universe::WorkflowV2DeliverableContract;
use serde_json::{Value, json};

pub fn task_schema() -> Value {
    let sample = FrozenTask {
        status: None,
        task_id: String::new(),
        file_name: String::new(),
        depends_on: vec![FrozenDependency {
            task_id: String::new(),
            ordering_only: false,
            consumes: vec![ConsumedArtifact {
                artifact_path: String::new(),
                kind: Some(String::new()),
                instance_source_records_field: Some(String::new()),
                registry_records_field: Some(String::new()),
            }],
        }],
        blocks: vec![String::new()],
        implements: vec![String::new()],
        source_sections: vec![String::new()],
        deliverable_contracts: vec![contract_sample()],
    };
    let mut schema = shape(&serde_json::to_value(sample).expect("FrozenTask serializes"));
    schema["required"] = json!(["task_id", "file_name"]);
    for field in ["blocks", "implements", "source_sections"] {
        schema["properties"][field]["uniqueItems"] = json!(true);
        schema["properties"][field]["items"]["pattern"] = json!(r"^\S(.*\S)?$");
    }
    let edge = &mut schema["properties"]["depends_on"];
    edge["uniqueItems"] = json!(true);
    edge["items"]["required"] = json!(["task_id"]);
    edge["items"]["properties"]["task_id"]["pattern"] = json!(r"^\S(.*\S)?$");
    edge["items"]["properties"]["consumes"]["items"]["required"] = json!(["artifact_path"]);
    let contract = &mut schema["properties"]["deliverable_contracts"]["items"];
    for field in [
        "required_universe",
        "registry_minimum_count",
        "series_overlap_min_rows",
    ] {
        contract["properties"]
            .as_object_mut()
            .expect("contract properties")
            .remove(field);
    }
    contract["required"] = json!(["kind", "artifact_path"]);
    for field in ["kind", "artifact_path"] {
        contract["properties"][field]["minLength"] = json!(1);
    }
    // Map samples carry one value so their accepted value type survives shaping.
    for field in [
        "minimum_count_fields",
        "registry_identity_fields",
        "response_identity_fields",
    ] {
        let map = &mut contract["properties"][field];
        let value = map["properties"][""].take();
        *map = json!({"type":"object", "additionalProperties":value});
    }
    schema
}

fn shape(sample: &Value) -> Value {
    match sample {
        Value::Object(fields) => json!({"type":"object", "properties":fields.iter()
            .map(|(key, value)| (key.clone(), shape(value))).collect::<serde_json::Map<_, _>>()}),
        Value::Array(items) => {
            json!({"type":"array", "items":items.first().map(shape).unwrap_or(json!({"type":"string"}))})
        }
        Value::Bool(_) => json!({"type":"boolean"}),
        Value::Number(_) => json!({"type":"integer"}),
        _ => json!({"type":"string"}),
    }
}

pub(super) fn task_clause() -> String {
    format!(
        "task (skeleton records only): {}; implements and/or deliverable_contracts must be non-empty, ",
        task_schema()
    )
}

// Exhaustive construction makes new accepted fields a compile-time hint update.
fn contract_sample() -> WorkflowV2DeliverableContract {
    WorkflowV2DeliverableContract {
        implements: Vec::new(),
        kind: String::new(),
        artifact_path: String::new(),
        typed_verifier_command: Some(String::new()),
        registry_path: Some(String::new()),
        instance_source_path: Some(String::new()),
        instance_source_records_field: Some(String::new()),
        instance_artifact_field: Some(String::new()),
        min_instances: 0,
        required_universe: false,
        data_kind: Some(String::new()),
        universe_fields: vec![String::new()],
        cells_field: Some(String::new()),
        cell_identity_fields: vec![String::new()],
        required_true_fields: vec![String::new()],
        required_nonempty_fields: vec![String::new()],
        positive_count_fields: vec![String::new()],
        minimum_count_fields: [(String::new(), 0)].into(),
        gaps_field: Some(String::new()),
        registry_records_field: Some(String::new()),
        registry_key_fields: vec![String::new()],
        registry_required_true_fields: vec![String::new()],
        registry_status_field: Some(String::new()),
        registry_allowed_statuses: vec![String::new()],
        registry_count_field: Some(String::new()),
        registry_minimum_count: 0,
        registry_identity_fields: [(String::new(), String::new())].into(),
        payload_path_field: Some(String::new()),
        payload_format: Some(String::new()),
        required_fields: vec![String::new()],
        non_constant_fields: vec![String::new()],
        artifact_format: Some(String::new()),
        observed_time_field: Some(String::new()),
        closed_weekdays: Some(vec![0]),
        closed_dates: vec![String::new()],
        step_variety_min_rows: Some(0),
        step_variety_min_percent: Some(0),
        series_value_fields: vec![String::new()],
        series_overlap_min_rows: 0,
        request_path_field: Some(String::new()),
        requested_count_field: Some(String::new()),
        response_path_field: Some(String::new()),
        response_identity_fields: [(String::new(), String::new())].into(),
        validation_path_field: Some(String::new()),
        validation_status_field: Some(String::new()),
        validation_checks_field: Some(String::new()),
        validation_check_status_field: Some(String::new()),
        validation_failed_values: vec![String::new()],
        validation_passed_values: vec![String::new()],
    }
}
