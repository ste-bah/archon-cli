//! Execution-path metadata of result-store records: values that say how a run
//! reached its result (admission order, the attempt of an interrupted call,
//! per-run host call ids), not what the result is.

/// Normalises one record. `interrupted` names the calls a fault cell
/// interrupted, by call id prefix (author calls) or by host `commandId`: a resume
/// runs such a call again, so its attempt number describes the fault, not the
/// result. Every other call keeps its attempt number and is compared exactly.
pub(super) fn normalize_lifecycle_metadata(value: &mut serde_json::Value, interrupted: &[&str]) {
    let host_call_id = value
        .get("call")
        .and_then(|call| {
            (call.get("method").and_then(serde_json::Value::as_str) == Some("hostCommand"))
                .then(|| call.get("id").and_then(serde_json::Value::as_str))
                .flatten()
        })
        .map(str::to_owned);
    if let Some(host_call_id) = host_call_id {
        replace_string_values(value, &host_call_id, "<host-call-id>");
        if let Some(call) = value
            .get_mut("call")
            .and_then(serde_json::Value::as_object_mut)
        {
            call.insert(
                "id".into(),
                serde_json::Value::String("<host-call-id>".into()),
            );
        }
    }

    match value {
        serde_json::Value::Object(map) => {
            // Resume adds an admission event, so the admission order
            // describes this run's execution path.
            map.remove("admission_sequence");
            if map
                .get("call")
                .is_some_and(|call| is_interrupted(call, interrupted))
            {
                map.remove("attempt");
            }
            for (key, nested) in map.iter_mut() {
                if key == "call_id" {
                    *nested = serde_json::Value::String("<call-id>".into());
                } else {
                    normalize_lifecycle_metadata(nested, interrupted);
                }
            }
        }
        serde_json::Value::Array(values) => {
            for nested in values {
                normalize_lifecycle_metadata(nested, interrupted);
            }
        }
        _ => {}
    }
}

fn is_interrupted(call: &serde_json::Value, interrupted: &[&str]) -> bool {
    let id = call.get("id").and_then(serde_json::Value::as_str);
    let command = call
        .pointer("/options/extra/commandId")
        .and_then(serde_json::Value::as_str);
    interrupted
        .iter()
        .any(|name| id.is_some_and(|id| id.starts_with(name)) || Some(*name) == command)
}

fn replace_string_values(value: &mut serde_json::Value, from: &str, to: &str) {
    match value {
        serde_json::Value::String(text) => *text = text.replace(from, to),
        serde_json::Value::Object(map) => {
            for nested in map.values_mut() {
                replace_string_values(nested, from, to);
            }
        }
        serde_json::Value::Array(values) => {
            for nested in values {
                replace_string_values(nested, from, to);
            }
        }
        _ => {}
    }
}
