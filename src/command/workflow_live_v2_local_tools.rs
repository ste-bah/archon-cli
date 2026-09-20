//! The three workflow-internal pseudo-tools a script may reach through
//! `w.tool(id, { tool })`: checkpoint, saveArtifact and requireArtifact.
//! Split out of `workflow_live_v2_host_dispatch.rs` to hold the 500-line
//! ceiling; the dispatcher routes `Tool` calls here after the acceptance
//! stage has had its look.

use super::*;

pub(crate) fn execute_declared_local_tool(
    execution: WorkflowV2CallExecution,
    v2_store: &WorkflowV2ResultStore,
    task_universe: Option<&WorkflowV2TaskUniverse>,
) -> archon_workflow::WorkflowResult<WorkflowV2Result> {
    let tool_name = declared_local_tool_name(&execution).ok_or_else(|| {
        WorkflowError::SpecInvalid(format!(
            "w.tool('{}') is missing required allowlisted local tool name in options.tool",
            execution.call.id
        ))
    })?;
    let method = allowlisted_local_tool_method(&tool_name).ok_or_else(|| {
        WorkflowError::SpecInvalid(format!(
            "w.tool('{}') declared unknown local tool '{}'; allowed generated V2 tools are checkpoint, saveArtifact, and requireArtifact",
            execution.call.id, tool_name
        ))
    })?;
    let delegated = WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            method,
            ..execution.call
        },
        input: execution.input,
        depends_on: execution.depends_on,
    };
    execute_local_host_call(&delegated, v2_store, task_universe)?.ok_or_else(|| {
        WorkflowError::SpecInvalid(format!(
            "w.tool('{}') could not execute allowlisted local tool '{}'",
            delegated.call.id, tool_name
        ))
    })
}

fn declared_local_tool_name(execution: &WorkflowV2CallExecution) -> Option<String> {
    execution
        .call
        .options
        .extra
        .get("tool")
        .or_else(|| execution.call.options.extra.get("name"))
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            execution
                .input
                .get("options")
                .and_then(|options| options.get("tool").or_else(|| options.get("name")))
                .and_then(serde_json::Value::as_str)
        })
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
}

fn allowlisted_local_tool_method(tool_name: &str) -> Option<WorkflowV2HostMethod> {
    match tool_name.trim().to_ascii_lowercase().as_str() {
        "checkpoint" => Some(WorkflowV2HostMethod::Checkpoint),
        "saveartifact" | "save_artifact" => Some(WorkflowV2HostMethod::SaveArtifact),
        "requireartifact" | "require_artifact" => Some(WorkflowV2HostMethod::RequireArtifact),
        _ => None,
    }
}
