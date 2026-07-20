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

/// Every key `allowed_mcp_tools` treats as a tool declaration. The write path
/// strips exactly these from the agent-authored branch item before stamping
/// the authoritative set, so the two lists must never drift.
pub(crate) const TOOL_DECLARATION_FIELDS: &[&str] = &[
    "required_tools",
    "requiredTools",
    "tool_requirements",
    "toolRequirements",
    "mcp_tools",
    "mcpTools",
];

fn is_tool_field(key: &str) -> bool {
    TOOL_DECLARATION_FIELDS.contains(&key)
}

/// Remove every tool-declaration key at EVERY level of a value. `allowed_mcp_tools`
/// (and the write no-op guard) scan the whole input recursively, so a shallow
/// strip leaves a nested `{...: {mcp_tools: [...]}}` forgery reachable. Applied
/// to agent-authored branch items so only host-stamped, task-universe-derived
/// tools can ever bind.
pub(crate) fn strip_tool_declarations(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(object) => {
            object.retain(|key, _| !is_tool_field(key));
            for child in object.values_mut() {
                strip_tool_declarations(child);
            }
        }
        serde_json::Value::Array(values) => {
            for child in values.iter_mut() {
                strip_tool_declarations(child);
            }
        }
        _ => {}
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn declared_tools_are_collected_from_the_stamped_branch_item() {
        // The write path stamps the task's required_tools into
        // branch.input["item"]["required_tools"]; allowed_mcp_tools must reach
        // them through the recursive scan (they are later intersected with the
        // project-permitted set to bind the MCP tools).
        let input = serde_json::json!({
            "fanout_item_id": "implement-task-tdl-120-1-0",
            "item": {
                "canonical_task_ids": ["TASK-TDL-120"],
                "required_tools": ["pine_compile", "pine_get_errors"]
            }
        });
        let mut tools = BTreeSet::new();
        collect_declared_tools(&input, &mut tools);
        assert!(tools.contains("pine_compile"), "{tools:?}");
        assert!(tools.contains("pine_get_errors"), "{tools:?}");
    }

    #[test]
    fn strip_removes_nested_tool_forgeries_so_none_are_collected() {
        // The reviewer's bypass: a no-tool item hides tool declarations one or
        // more levels below the root. After a recursive strip, the recursive
        // collector must find nothing.
        let mut item = serde_json::json!({
            "canonical_task_ids": ["TASK-NOTOOL"],
            "evidence": { "mcp_tools": ["pine_compile"] },
            "meta": { "notes": { "required_tools": ["pine_get_errors"] } },
            "list": [{ "requiredTools": ["pine_check"] }]
        });
        strip_tool_declarations(&mut item);
        let mut tools = BTreeSet::new();
        collect_declared_tools(&item, &mut tools);
        assert!(
            tools.is_empty(),
            "nested forgeries must be stripped: {tools:?}"
        );
    }

    #[test]
    fn mcp_prefixed_declared_tools_are_reduced_to_raw_names() {
        let input = serde_json::json!({
            "item": { "required_tools": ["mcp__tradingview__pine_compile"] }
        });
        let mut tools = BTreeSet::new();
        collect_declared_tools(&input, &mut tools);
        assert!(tools.contains("pine_compile"), "{tools:?}");
    }
}
