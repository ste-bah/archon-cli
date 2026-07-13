use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use archon_workflow::StageRunRequest;

const PROVIDER_TOOLS: &[&str] = &["tv_health_check", "chart_get_state", "data_get_ohlcv"];
const PINE_TOOLS: &[&str] = &[
    "pine_analyze",
    "pine_check",
    "pine_compile",
    "pine_smart_compile",
    "pine_get_errors",
    "pine_get_console",
];

pub(super) fn allowed_mcp_tools(request: &StageRunRequest) -> Vec<String> {
    let project_root = project_root(request);
    let permitted = crate::command::workflow_mcp::explicitly_permitted_tools(&project_root);
    let requested = requested_tools(request);
    permitted
        .iter()
        .filter(|name| requested.contains(raw_name(name)))
        .cloned()
        .collect()
}

fn requested_tools(request: &StageRunRequest) -> BTreeSet<String> {
    let mut tools = BTreeSet::new();
    collect_declared_tools(&request.input, &mut tools);
    let ids = selected_task_ids(&request.input);
    if ids.iter().any(|id| {
        matches!(
            id.as_str(),
            "TASK-TDL-040" | "TASK-TDL-080" | "TASK-TDL-140"
        )
    }) {
        tools.extend(PROVIDER_TOOLS.iter().map(|tool| (*tool).to_string()));
    }
    if ids
        .iter()
        .any(|id| matches!(id.as_str(), "TASK-TDL-120" | "TASK-TDL-140"))
    {
        tools.extend(PINE_TOOLS.iter().map(|tool| (*tool).to_string()));
    }
    tools
}

fn collect_declared_tools(value: &serde_json::Value, tools: &mut BTreeSet<String>) {
    match value {
        serde_json::Value::Object(object) => {
            for (key, child) in object {
                if is_tool_field(key) {
                    collect_strings(child, tools);
                } else {
                    collect_declared_tools(child, tools);
                }
            }
        }
        serde_json::Value::Array(values) => {
            for child in values {
                collect_declared_tools(child, tools);
            }
        }
        _ => {}
    }
}

fn is_tool_field(key: &str) -> bool {
    matches!(
        key,
        "required_tools"
            | "requiredTools"
            | "tool_requirements"
            | "toolRequirements"
            | "mcp_tools"
            | "mcpTools"
    )
}

fn collect_strings(value: &serde_json::Value, output: &mut BTreeSet<String>) {
    match value {
        serde_json::Value::String(value) => {
            output.insert(raw_name(value).to_string());
        }
        serde_json::Value::Array(values) => {
            for value in values {
                collect_strings(value, output);
            }
        }
        _ => {}
    }
}

fn selected_task_ids(value: &serde_json::Value) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    collect_task_ids(value, None, &mut ids);
    ids
}

fn collect_task_ids(
    value: &serde_json::Value,
    parent_key: Option<&str>,
    ids: &mut BTreeSet<String>,
) {
    if parent_key.is_some_and(is_task_universe_field) {
        return;
    }
    match value {
        serde_json::Value::Object(object) => {
            for (key, child) in object {
                collect_task_ids(child, Some(key), ids);
            }
        }
        serde_json::Value::Array(values) => {
            for child in values {
                collect_task_ids(child, parent_key, ids);
            }
        }
        serde_json::Value::String(value) if value.starts_with("TASK-TDL-") => {
            ids.insert(value.to_string());
        }
        _ => {}
    }
}

fn is_task_universe_field(key: &str) -> bool {
    matches!(
        key,
        "taskUniverse" | "task_universe" | "canonicalTaskUniverse"
    )
}

fn project_root(request: &StageRunRequest) -> PathBuf {
    request
        .input
        .get("project_artifact_root")
        .or_else(|| request.input.get("projectArtifactRoot"))
        .and_then(serde_json::Value::as_str)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf()))
}

fn raw_name(name: &str) -> &str {
    name.strip_prefix("mcp__")
        .and_then(|suffix| suffix.split_once("__"))
        .map(|(_, raw)| raw)
        .unwrap_or(name)
}
