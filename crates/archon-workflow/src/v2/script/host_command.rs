//! Shared parsing for raw `w.hostCommand` calls.

use super::*;

const ALLOWED_HOST_COMMAND_OPTIONS: &[&str] = &["commandId", "stdin", "target"];

pub fn parse_host_command_request(
    request: &ScriptHostRequest,
) -> WorkflowResult<HostCommandRequest> {
    let object = request.options.as_object().ok_or_else(|| {
        WorkflowError::SpecInvalid(
            "hostCommand options must be an object containing commandId and optional stdin and target"
                .to_string(),
        )
    })?;
    for key in object.keys() {
        if !ALLOWED_HOST_COMMAND_OPTIONS.contains(&key.as_str()) {
            return Err(WorkflowError::SpecInvalid(format!(
                "hostCommand option `{key}` is forbidden: executable, argv, cwd, environment, limits, destinations, write sets, and reuse identity are host-owned"
            )));
        }
    }
    let command_id = object
        .get("commandId")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            WorkflowError::SpecInvalid(
                "hostCommand requires a non-empty command capability id".to_string(),
            )
        })?;
    let stdin = match object.get("stdin") {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(value)) => Some(value.clone()),
        Some(_) => {
            return Err(WorkflowError::SpecInvalid(
                "hostCommand stdin must be a string or null".to_string(),
            ));
        }
    };
    let mut parsed = HostCommandRequest::new(command_id, stdin)?;
    parsed.target = match object.get("target") {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::Object(target)) => {
            for key in target.keys() {
                if !["taskId", "fileName"].contains(&key.as_str()) {
                    return Err(WorkflowError::SpecInvalid(format!(
                        "hostCommand target field `{key}` is forbidden; only taskId and fileName are allowed"
                    )));
                }
            }
            let field = |name: &str| {
                target
                    .get(name)
                    .and_then(serde_json::Value::as_str)
                    .filter(|value| !value.trim().is_empty())
                    .map(str::to_string)
                    .ok_or_else(|| {
                        WorkflowError::SpecInvalid(format!(
                            "hostCommand target requires a non-empty string `{name}`"
                        ))
                    })
            };
            Some(crate::v2::host_command::HostCommandSubject {
                task_id: field("taskId")?,
                file_name: field("fileName")?,
            })
        }
        Some(_) => {
            return Err(WorkflowError::SpecInvalid(
                "hostCommand target must be an object with taskId and fileName".to_string(),
            ));
        }
    };
    Ok(parsed)
}
