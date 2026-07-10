use serde_json::{Map, Value};

use super::WorkflowV2AgentRequest;

pub(super) fn normalize_agent_output(
    request: &WorkflowV2AgentRequest,
    output: &str,
) -> serde_json::Result<Value> {
    let mut value: Value = serde_json::from_str(output)?;
    let Some(object) = value.as_object_mut() else {
        return Ok(value);
    };
    stamp_envelope(request, object);
    stamp_artifact_ids(object);
    normalize_commands(object);
    Ok(value)
}

fn stamp_envelope(request: &WorkflowV2AgentRequest, object: &mut Map<String, Value>) {
    insert_missing(object, "id", Value::String(request.call.id.clone()));
    insert_missing(object, "stage", Value::String(request.call.id.clone()));
    insert_missing(
        object,
        "attempt",
        serde_json::json!(request_attempt(request)),
    );
    if let Some(run_id) = &request.project_artifacts.run_id {
        insert_missing(object, "workflow_id", Value::String(run_id.clone()));
    }
}

fn request_attempt(request: &WorkflowV2AgentRequest) -> u64 {
    request
        .input
        .get("attempt")
        .and_then(Value::as_u64)
        .unwrap_or(1)
}

fn stamp_artifact_ids(object: &mut Map<String, Value>) {
    let Some(artifacts) = object.get_mut("artifacts").and_then(Value::as_array_mut) else {
        return;
    };
    for (index, artifact) in artifacts.iter_mut().enumerate() {
        let Some(fields) = artifact.as_object_mut() else {
            continue;
        };
        if fields.get("id").is_some_and(value_present) {
            continue;
        }
        let path = fields
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or("artifact");
        fields.insert("id".to_string(), Value::String(artifact_id(index, path)));
    }
}

fn normalize_commands(object: &mut Map<String, Value>) {
    let Some(commands) = object.get_mut("commands_run").and_then(Value::as_array_mut) else {
        return;
    };
    for command in commands {
        let Some(fields) = command.as_object_mut() else {
            continue;
        };
        insert_missing(fields, "kind", Value::String("other".to_string()));
    }
}

fn artifact_id(index: usize, path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or("artifact");
    let safe: String = name
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect();
    format!("artifact-{index}-{}", safe.trim_matches('-'))
}

fn insert_missing(object: &mut Map<String, Value>, key: &str, value: Value) {
    if !object.get(key).is_some_and(value_present) {
        object.insert(key.to_string(), value);
    }
}

fn value_present(value: &Value) -> bool {
    !value.is_null() && value.as_str().is_none_or(|value| !value.trim().is_empty())
}
