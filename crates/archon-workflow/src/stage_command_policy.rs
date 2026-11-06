//! Whether a legacy YAML stage may run shell commands.
//!
//! The predicate decides both the agent's tool access and whether the stage
//! prompt carries command-execution guidance, so both callers have to read the
//! same answer. It is a pure function of the [`StageRunRequest`] this crate
//! owns: shell access requires a declared capability, never task or input prose.
//! Legacy YAML stages declare `commands: true` or a Bash/shell tool in their
//! flattened StageSpec extras; the context builder carries those as stage_extra.

use crate::{StageKind, StageRunRequest};

pub fn command_execution_stage(request: &StageRunRequest) -> bool {
    if stage_extra_requests_bash(&request.input)
        || matches!(request.stage_kind, StageKind::Implementation)
    {
        return true;
    }
    let Some(call) = request.input.get("v2_call") else {
        return false;
    };
    call.get("method").and_then(serde_json::Value::as_str) == Some("implementation")
        || call
            .get("runs_commands")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
}

/// Read the admitted typed call, never script input or an author's call id.
/// Lifecycle ids are host-owned only when `host_planned` was set by the
/// lifecycle port's task-local scope (script options cannot set that flag).
pub fn v2_call_runs_commands(call: &crate::WorkflowV2HostCall) -> bool {
    call.options.item_kind.as_deref() == Some("focused_verification")
        || (call.options.host_planned && verification_wave_id(&call.id))
}

/// Whether the host granted this stage a READ-ONLY shell (a review map
/// branch, REM-16): read-only tools plus Bash, never full tool access.
pub fn read_only_shell_grant(input: &serde_json::Value) -> bool {
    input
        .get("stage_extra")
        .and_then(|extra| extra.get("read_only_shell"))
        .and_then(serde_json::Value::as_bool)
        == Some(true)
}

fn verification_wave_id(stage_id: &str) -> bool {
    let id = stage_id.to_ascii_lowercase().replace('-', "_");
    id.starts_with("verification_wave_") || id.starts_with("review_verification_wave_")
}

fn stage_extra_requests_bash(input: &serde_json::Value) -> bool {
    (input.get("v2_call").is_none()
        && input
            .get("stage_extra")
            .and_then(|extra| extra.get("commands"))
            .and_then(serde_json::Value::as_bool)
            == Some(true))
        || declared_bash_tool(input)
}

fn declared_bash_tool(input: &serde_json::Value) -> bool {
    let Some(extra) = input.get("stage_extra") else {
        return false;
    };
    ["allowed_tools", "tools", "required_tools"]
        .iter()
        .filter_map(|key| extra.get(*key))
        .flat_map(text_values)
        .any(|tool| tool.eq_ignore_ascii_case("bash") || tool.eq_ignore_ascii_case("shell"))
}

fn text_values(value: &serde_json::Value) -> Vec<&str> {
    match value {
        serde_json::Value::String(value) => vec![value.as_str()],
        serde_json::Value::Array(values) => values
            .iter()
            .filter_map(serde_json::Value::as_str)
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
#[path = "stage_command_policy_tests.rs"]
mod tests;
