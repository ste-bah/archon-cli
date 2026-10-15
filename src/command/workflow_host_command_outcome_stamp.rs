//! Stamp persisted host-command outcomes with the logic that judged them.

use archon_workflow::{WorkflowResult, WorkflowV2HostCall, WorkflowV2HostMethod};

pub(crate) fn stamp_host_outcome(
    executor: Option<&dyn crate::command::workflow_host_command_exec::WorkflowHostCommandExecutor>,
    call: &WorkflowV2HostCall,
    mut data: serde_json::Value,
) -> WorkflowResult<serde_json::Value> {
    if call.method != WorkflowV2HostMethod::HostCommand {
        return Ok(data);
    }
    let (Some(executor), Some(request)) = (executor, call.options.host_command.as_ref()) else {
        return Ok(data);
    };
    let limits = executor.limits_fingerprint(request)?;
    let logic = executor.logic_version(request)?;
    let digest = executor.logic_digest(request)?;
    let build = executor.logic_build(request)?;
    if let Some(map) = data.as_object_mut() {
        if let Some(stamp) = limits {
            map.insert(
                crate::command::workflow_host_command_exec::identity::LIMITS_FINGERPRINT.into(),
                stamp,
            );
        }
        if let Some(version) = logic {
            map.insert(
                crate::command::workflow_host_command_logic::LOGIC_VERSION_STAMP.into(),
                serde_json::json!(version),
            );
        }
        if let Some(digest) = digest {
            map.insert(
                crate::command::workflow_host_command_logic::LOGIC_DIGEST_STAMP.into(),
                serde_json::json!(digest),
            );
        }
        if let Some(build) = build {
            map.insert(
                crate::command::workflow_host_command_logic::LOGIC_BUILD_STAMP.into(),
                serde_json::json!(build),
            );
        }
    }
    Ok(data)
}
