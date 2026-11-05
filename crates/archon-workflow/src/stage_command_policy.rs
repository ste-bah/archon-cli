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
            .get("id")
            .and_then(serde_json::Value::as_str)
            .is_some_and(verification_wave_id)
}

/// Whether a typed read-only V2 call is granted commands: by the host's
/// stage grant (a review map branch, REM-16) or as a verification wave. The
/// agent prompt reads this to say which shell the stage has, so what it is
/// told and what it gets are the same answer.
pub fn v2_call_runs_commands(call_id: &str, input: &serde_json::Value) -> bool {
    stage_extra_requests_bash(input) || verification_wave_id(call_id)
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
    let Some(extra) = input.get("stage_extra") else {
        return false;
    };
    extra.get("commands").and_then(serde_json::Value::as_bool) == Some(true)
        || ["allowed_tools", "tools", "required_tools"]
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
