//! The task schema and hint share the serialized FrozenTask shape.
use crate::task_skeleton::{ConsumedArtifact, FrozenDependency, FrozenTask};
use crate::task_universe::WorkflowV2DeliverableContract;
use serde_json::{Value, json};

pub fn task_schema() -> Value {
    let sample = FrozenTask {
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
        deliverable_contracts: vec![WorkflowV2DeliverableContract {
            instance_source_records_field: Some(String::new()),
            registry_records_field: Some(String::new()),
            ..Default::default()
        }],
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
