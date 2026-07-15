use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use archon_workflow::StageRunRequest;

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
