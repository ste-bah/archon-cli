use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use archon_workflow::StageRunRequest;
// The key vocabulary and its scrub live in archon-workflow so the collector
// here and the write layer's strip cannot drift apart.
use archon_workflow::tool_declarations::{is_tool_field, raw_tool_name as raw_name};

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

/// Declared tool names that are native Archon tools, not project MCP tools.
///
/// Issue-28, run wf-719ff3b0 stage agents-12 (TASK-AHDM-001): the task declared
/// `required_tools: ["memory_recall"]`, the acceptance check
/// (`agent_adapter_a::unexercised_required_tools`) demanded that name in
/// `commands_run`, and the coder was never offered it — `allowed_tools` was a
/// fixed per-stage list plus [`allowed_mcp_tools`], and a declared name that is
/// not `mcp__`-qualified matched neither. Every task requiring a native tool
/// was rejected as "never exercised" after the repair loop and its dependants
/// were blocked, while the coder had done the honest equivalent through Bash.
///
/// Keyed only on what the task declared, what `granted` already holds, and the
/// stage's access level. Deliberately NOT filtered against a native-tool list:
/// the registry decides whether a name exists (`clone_filtered` drops the rest)
/// and the "never exercised" check remains the honest backstop when it does not.
///
/// Read-only stages stay read-only: a declared name reaches one only if it is
/// in the adapter's read-only vocabulary, so a task cannot promote a reviewer
/// to `memory_store` or `Write` by declaring it.
pub(super) fn declared_native_tools(request: &StageRunRequest, granted: &[String]) -> Vec<String> {
    let full_access = super::workflow_live_runner::full_tool_access(request);
    let mut declared = BTreeSet::new();
    visit_declared_tools(&request.input, &mut |name| {
        let name = name.trim();
        // `raw_name` is the scrub that reduces BOTH qualifier conventions
        // (`mcp__server__x`, `mcp_action:x`); a name it leaves untouched carries
        // no MCP qualifier and is therefore a native declaration. The literal
        // "starts with mcp__" test would let `mcp_action:x` through as native.
        if !name.is_empty() && raw_name(name) == name {
            declared.insert(name.to_string());
        }
    });
    declared
        .into_iter()
        .filter(|name| !granted.iter().any(|tool| tool == name))
        // A bare `lint_compile` that [`allowed_mcp_tools`] already bound to
        // `mcp__vendor__lint_compile` is that MCP tool, not a second native
        // one; offering the bare spelling too would put a name in the contract
        // the registry cannot serve.
        .filter(|name| {
            !granted
                .iter()
                .any(|tool| tool.starts_with("mcp__") && raw_name(tool) == name)
        })
        .filter(|name| full_access || archon_pipeline::subagent_adapter::is_read_only_tool(name))
        .collect()
}

fn requested_tools(request: &StageRunRequest) -> BTreeSet<String> {
    let mut tools = BTreeSet::new();
    collect_declared_tools(&request.input, &mut tools);
    tools
}

fn collect_declared_tools(value: &serde_json::Value, tools: &mut BTreeSet<String>) {
    visit_declared_tools(value, &mut |name| {
        tools.insert(raw_name(name).to_string());
    });
}

/// Every string under a tool-declaration field, verbatim, wherever it sits in
/// the input. One walk for both collectors so the MCP binding and the native
/// admission cannot disagree about where declarations live.
fn visit_declared_tools(value: &serde_json::Value, visit: &mut dyn FnMut(&str)) {
    match value {
        serde_json::Value::Object(object) => {
            for (key, child) in object {
                if is_tool_field(key) {
                    visit_strings(child, visit);
                } else {
                    visit_declared_tools(child, visit);
                }
            }
        }
        serde_json::Value::Array(values) => {
            for child in values {
                visit_declared_tools(child, visit);
            }
        }
        _ => {}
    }
}

fn visit_strings(value: &serde_json::Value, visit: &mut dyn FnMut(&str)) {
    match value {
        serde_json::Value::String(value) => visit(value),
        serde_json::Value::Array(values) => {
            for value in values {
                visit_strings(value, visit);
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

#[cfg(test)]
mod tests {
    use super::*;
    use archon_workflow::tool_declarations::strip_tool_declarations;
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
                "required_tools": ["lint_compile", "lint_get_errors"]
            }
        });
        let mut tools = BTreeSet::new();
        collect_declared_tools(&input, &mut tools);
        assert!(tools.contains("lint_compile"), "{tools:?}");
        assert!(tools.contains("lint_get_errors"), "{tools:?}");
    }

    #[test]
    fn strip_removes_nested_tool_forgeries_so_none_are_collected() {
        // The reviewer's bypass: a no-tool item hides tool declarations one or
        // more levels below the root. After a recursive strip, the recursive
        // collector must find nothing.
        let mut item = serde_json::json!({
            "canonical_task_ids": ["TASK-NOTOOL"],
            "evidence": { "mcp_tools": ["lint_compile"] },
            "meta": { "notes": { "required_tools": ["lint_get_errors"] } },
            "list": [{ "requiredTools": ["lint_check"] }]
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
            "item": { "required_tools": ["mcp__vendor__lint_compile"] }
        });
        let mut tools = BTreeSet::new();
        collect_declared_tools(&input, &mut tools);
        assert!(tools.contains("lint_compile"), "{tools:?}");
    }

    fn implementation_request(required: serde_json::Value) -> StageRunRequest {
        crate::command::workflow_live::workflow_live_test_support::request(serde_json::json!({
            "item": { "canonical_task_ids": ["TASK-AHDM-001"], "required_tools": required }
        }))
    }

    fn read_only_request(required: serde_json::Value) -> StageRunRequest {
        let mut request = implementation_request(required);
        request.stage_id = "review".into();
        request.stage_kind = archon_workflow::StageKind::Agent;
        // No verification vocabulary: `command_execution_stage` sniffs prose
        // for it and would promote the stage to a shell.
        request.task = "Review the diff for correctness.".into();
        request
    }

    #[test]
    fn issue_28_declared_native_tool_is_admitted_on_an_implementation_stage() {
        // wf-719ff3b0 agents-12: `required_tools: ["memory_recall"]` never
        // reached the coder's contract because it is not an MCP name.
        let request = implementation_request(serde_json::json!(["memory_recall"]));
        let tools = declared_native_tools(&request, &["Read".to_string()]);
        assert_eq!(tools, vec!["memory_recall".to_string()]);
    }

    #[test]
    fn mcp_qualified_declarations_are_not_native_tools() {
        // Both qualifier conventions stay on the MCP path; neither is native.
        let request = implementation_request(serde_json::json!([
            "mcp__vendor__lint_check",
            "mcp_action:tv_health_check"
        ]));
        assert!(declared_native_tools(&request, &[]).is_empty());
    }

    #[test]
    fn bare_name_already_bound_to_a_project_mcp_tool_is_not_duplicated() {
        let request = implementation_request(serde_json::json!(["lint_compile", "memory_recall"]));
        let granted = vec!["mcp__vendor__lint_compile".to_string()];
        assert_eq!(
            declared_native_tools(&request, &granted),
            vec!["memory_recall".to_string()]
        );
    }

    #[test]
    fn duplicates_whitespace_empties_and_already_granted_names_collapse() {
        let request = implementation_request(serde_json::json!([
            " memory_recall ",
            "memory_recall",
            "",
            "   ",
            "Bash",
            "memory_store"
        ]));
        let granted = vec!["Bash".to_string()];
        assert_eq!(
            declared_native_tools(&request, &granted),
            vec!["memory_recall".to_string(), "memory_store".to_string()]
        );
    }

    #[test]
    fn read_only_stage_admits_only_read_only_vocabulary() {
        // memory_recall is in the adapter's READ_ONLY_TOOLS; memory_store is
        // not, and a declaration must not promote a reviewer to a writer.
        let request = read_only_request(serde_json::json!([
            "memory_recall",
            "memory_store",
            "Write"
        ]));
        assert_eq!(
            declared_native_tools(&request, &[]),
            vec!["memory_recall".to_string()]
        );
    }
}
