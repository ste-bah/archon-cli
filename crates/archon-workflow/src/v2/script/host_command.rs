//! Shared parsing for raw `w.hostCommand` calls.

use super::*;

const ALLOWED_HOST_COMMAND_OPTIONS: &[&str] = &["commandId", "stdin"];

pub fn parse_host_command_request(
    request: &ScriptHostRequest,
) -> WorkflowResult<HostCommandRequest> {
    let object = request.options.as_object().ok_or_else(|| {
        WorkflowError::SpecInvalid(
            "hostCommand options must be an object containing commandId and optional stdin"
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
    HostCommandRequest::new(command_id, stdin)
}
