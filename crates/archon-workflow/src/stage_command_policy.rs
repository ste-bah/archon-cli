//! Whether a legacy YAML stage may run shell commands.
//!
//! The predicate decides both the agent's tool access and whether the stage
//! prompt carries command-execution guidance, so both callers have to read the
//! same answer. It is a pure function of the [`StageRunRequest`] this crate
//! owns: declared capabilities decide access independently of task prose.

use crate::StageRunRequest;

pub fn command_execution_stage(request: &StageRunRequest) -> bool {
    v2_call_runs_commands(&request.stage_id, &request.input)
}

/// Shell access comes from an explicit grant or declared verification semantics.
/// Unannotated recorded calls retain the old verification-prefix interpretation.
pub fn v2_call_runs_commands(call_id: &str, input: &serde_json::Value) -> bool {
    use crate::v2::call_metadata::CallSemantics;
    if stage_extra_requests_bash(input) || read_only_shell_grant(input) {
        return true;
    }
    if let Some(value) = input.get("v2_call") {
        return serde_json::from_value::<crate::v2::host_api::WorkflowV2HostCall>(value.clone())
            .is_ok_and(|call| call.verification());
    }
    call_id.verification()
}

/// Whether the host granted this stage a read-only shell.
pub fn read_only_shell_grant(input: &serde_json::Value) -> bool {
    input
        .get("stage_extra")
        .and_then(|extra| extra.get("read_only_shell"))
        .and_then(serde_json::Value::as_bool)
        == Some(true)
}

fn stage_extra_requests_bash(input: &serde_json::Value) -> bool {
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
