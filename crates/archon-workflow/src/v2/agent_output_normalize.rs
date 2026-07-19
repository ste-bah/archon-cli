use serde_json::{Map, Value};

use super::WorkflowV2AgentRequest;

pub(super) fn normalize_agent_output(
    request: &WorkflowV2AgentRequest,
    output: &str,
) -> serde_json::Result<Value> {
    let mut value: Value = parse_envelope_document(output)?;
    let Some(object) = value.as_object_mut() else {
        return Ok(value);
    };
    stamp_envelope(request, object);
    normalize_path_records(object, "artifacts");
    normalize_path_records(object, "files_read");
    normalize_path_records(object, "files_changed");
    stamp_artifact_ids(object);
    normalize_commands(object);
    Ok(value)
}

/// Parse the agent reply as one JSON envelope. Providers routinely wrap an
/// otherwise-valid envelope in markdown fences or prose; when the whole reply
/// is not bare JSON, accept it only if it contains exactly one complete
/// top-level JSON object carrying a `status` member. Location only — no
/// content is invented, ambiguity stays a loud failure, the forbidden-text
/// guard has already seen the full raw reply, and every schema and validation
/// gate still runs on whatever parses here.
fn parse_envelope_document(output: &str) -> serde_json::Result<Value> {
    let root_error = match serde_json::from_str(output.trim()) {
        Ok(value) => return Ok(value),
        Err(error) => error,
    };
    // Tolerate EXACTLY ONE complete top-level object wrapped in fences or
    // prose. Two or more complete objects is an ambiguous reply (echoed
    // schema example + real envelope, draft + final): never guess which is
    // the envelope — surface the root error so the repair loop re-asks.
    let mut found: Option<Value> = None;
    let mut skip_until = 0;
    for (index, _) in output.match_indices(['{', '[']) {
        if index < skip_until {
            continue;
        }
        let mut stream = serde_json::Deserializer::from_str(&output[index..]).into_iter::<Value>();
        match stream.next() {
            Some(Ok(value)) => {
                skip_until = index + stream.byte_offset();
                // A complete array is another JSON document, not prose. In
                // particular, never extract a validating envelope nested in
                // a one-element array and pretend it was top-level.
                if value.is_array() {
                    return Err(root_error);
                }
                if !value.is_object() {
                    continue;
                }
                if found.is_some() {
                    return Err(root_error);
                }
                found = Some(value);
            }
            // An unterminated object is a truncation signature: the reply is
            // structurally incomplete, and any complete object inside it (an
            // echoed branch envelope in data.items, a coverage entry) could
            // impersonate the real reply. Never extract from a truncated
            // reply. (Prose braces fail with non-EOF errors and fall through.)
            Some(Err(error)) if error.is_eof() || starts_like_json_container(&output[index..]) => {
                return Err(root_error);
            }
            _ => {}
        }
    }
    // Every envelope declares `status`; a lone complete NESTED object inside
    // a truncated envelope must not impersonate the reply. Evidence items
    // lack `status`; task_coverage entries carry `status` AND `task_id` —
    // and `task_id` is never a top-level envelope key, so its presence marks
    // a fragment.
    match found {
        Some(value) if value.get("status").is_some() && value.get("task_id").is_none() => Ok(value),
        _ => Err(root_error),
    }
}

fn starts_like_json_container(candidate: &str) -> bool {
    let mut chars = candidate.chars();
    match chars.next() {
        // Valid JSON object members begin with a quoted key. If such a
        // container is malformed later, no complete object inside it may be
        // promoted to the reply envelope. Natural-language braces such as
        // "{ curly braces" remain eligible prose.
        Some('{') => matches!(chars.find(|ch| !ch.is_whitespace()), Some('"' | '}')),
        // Arrays are never result envelopes. A malformed array can still
        // contain one complete validating object, so it is always ambiguous.
        Some('[') => true,
        _ => false,
    }
}

fn normalize_path_records(object: &mut Map<String, Value>, field: &str) {
    let Some(records) = object.get_mut(field).and_then(Value::as_array_mut) else {
        return;
    };
    for record in records {
        let Some(path) = record.as_str() else {
            continue;
        };
        *record = serde_json::json!({"path": path});
    }
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
        normalize_command_status(fields);
    }
}

fn normalize_command_status(fields: &mut Map<String, Value>) {
    let Some(status) = fields.get("status").and_then(Value::as_str) else {
        return;
    };
    let canonical = match status.to_ascii_lowercase().as_str() {
        "passed" | "ok" | "success" => "succeeded",
        "failure" | "error" => "failed",
        "skip" => "skipped",
        _ => return,
    };
    fields.insert("status".to_string(), Value::String(canonical.to_string()));
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
