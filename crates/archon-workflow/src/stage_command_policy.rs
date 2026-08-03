//! Whether a legacy YAML stage may run shell commands.
//!
//! The predicate decides both the agent's tool access and whether the stage
//! prompt carries command-execution guidance, so both callers have to read the
//! same answer. It is a pure function of the [`StageRunRequest`] this crate
//! owns: typed V2 calls decide from declared fields, and prose sniffing is
//! reserved for requests that carry no typed call.

use crate::StageRunRequest;

pub fn command_execution_stage(request: &StageRunRequest) -> bool {
    if stage_extra_requests_bash(request) {
        return true;
    }
    // Typed V2 calls decide from declared fields: focused-verification waves
    // run commands; every other read-only call gets no shell. Prose sniffing
    // is reserved for requests without a typed call.
    if request.input.get("v2_call").is_some() {
        let id = request.stage_id.to_ascii_lowercase().replace('-', "_");
        if id.starts_with("verification_wave_") || id.starts_with("review_verification_wave_") {
            return true;
        }
        if generated_v2_read_only_call(request) {
            return false;
        }
    }
    if command_execution_stage_id(&request.stage_id) {
        return true;
    }
    let haystack = format!(
        "{}\n{}\n{}\n{}",
        request.stage_id,
        request.task,
        request
            .input
            .get("stage_task")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default(),
        request.input
    )
    .to_ascii_lowercase();
    command_execution_text(&haystack)
}

fn generated_v2_read_only_call(request: &StageRunRequest) -> bool {
    let Some(v2_call) = request.input.get("v2_call") else {
        return false;
    };
    let write_mode = v2_call.get("write_mode");
    if write_mode.is_some_and(|value| !value.is_null()) {
        return false;
    }
    let method = v2_call
        .get("method")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    matches!(
        method,
        "agent" | "fanout" | "parallel" | "reduce" | "finalReport" | "qualityGate" | "humanGate"
    )
}

fn command_execution_stage_id(stage_id: &str) -> bool {
    let id = stage_id.to_ascii_lowercase().replace('-', "_");
    id.starts_with("verification_wave_")
        || id.starts_with("review_verification_wave_")
        || id.ends_with("_tests")
        || id.contains("_post_tests")
        || id.contains("_focused_tests")
        || id.contains("_verification")
}

fn command_execution_text(haystack: &str) -> bool {
    [
        "focused_test",
        "focused-test",
        "focused test",
        "focused tests",
        "post-remediation tests",
        "post remediation tests",
        "cargo test",
        "test command",
        "test execution",
        "test evidence",
        "run tests",
        "run focused",
        "tests and checks",
        "verification",
        "verify",
        "quality gate",
        "cargo check",
        "cargo build",
        "cargo fmt",
        "rustfmt",
        "clippy",
        "lint",
    ]
    .iter()
    .any(|needle| haystack.contains(needle))
}

fn stage_extra_requests_bash(request: &StageRunRequest) -> bool {
    let Some(extra) = request.input.get("stage_extra") else {
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
