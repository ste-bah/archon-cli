//! Task-local MCP obligations checked before body publication and at set gates.
//! HTTP/service adapters are not MCP clients; no ambient tool union is granted.

use std::collections::BTreeSet;
use std::path::Path;
use archon_workflow::task_universe::{WorkflowV2TaskUniverseTask, parsing::parse_task_file, task_files_under};
use archon_workflow::tool_declarations::raw_tool_name;
use crate::command::workflow_gate::{GateFinding, GateId};

pub(super) fn inspect(project: &Path, task: &WorkflowV2TaskUniverseTask, raw: &str) -> Vec<String> {
    let permitted = crate::command::workflow_mcp::explicitly_permitted_tools(project);
    let granted: BTreeSet<_> = permitted.iter().filter(|name| {
        task.required_tools.iter().any(|tool| matches_tool(tool, name))
    }).cloned().collect();
    let mut defects = BTreeSet::new();
    for contract in &task.deliverable_contracts {
        let kind = contract.kind.to_ascii_lowercase();
        let parts: Vec<_> = kind.split(|c: char| !c.is_alphanumeric()).collect();
        if !parts.contains(&"mcp") { continue; }
        let server = parts.iter().position(|part| *part == "mcp")
            .filter(|index| *index > 0).map(|index| parts[..index].join("-"));
        let candidates: Vec<_> = permitted.iter().filter(|name| {
            server.as_ref().is_none_or(|server| name.starts_with(&format!("mcp__{server}__")))
        }).collect();
        if !candidates.iter().any(|name| granted.contains(*name)) {
            defects.insert(format!(
                "{}: required_tools does not grant the MCP access declared by contract '{}'; declare the exact project-permitted MCP tools this task must exercise, using .mcp.json. Available matching tools: {}. If this is an HTTP or offline adapter, correct its contract rather than declaring unrelated tools.",
                task.canonical_task_id, contract.kind,
                candidates.iter().map(|name| name.as_str()).collect::<Vec<_>>().join(", ")
            ));
        }
    }
    // Only exact tool identifiers in Focused Tests are invocation obligations.
    // Mentions in optional design/review prose must not become mandatory calls.
    let mut focused = false;
    for line in raw.lines() {
        if line.starts_with("## ") {
            focused = line.trim().eq_ignore_ascii_case("## Focused Tests");
            continue;
        }
        if !focused { continue; }
        for token in line.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == ':' || c == '-')) {
            let named = permitted.iter().find(|name| matches_tool(token, name));
            if token.starts_with("mcp__") || token.starts_with("mcp_action:") || named.is_some() {
                if !named.is_some_and(|name| granted.contains(name)) {
                    defects.insert(format!("{}: required_tools is missing a permitted grant for focused MCP call '{token}'; use an exact permitted name from .mcp.json and declare that invocation", task.canonical_task_id));
                }
            }
        }
    }
    for tool in task.required_tools.iter().filter(|tool| tool.starts_with("mcp__") || tool.starts_with("mcp_action:")) {
        if !permitted.iter().any(|name| matches_tool(tool, name)) {
            defects.insert(format!("{}: required_tools names MCP tool '{tool}' which is not permitted by the project .mcp.json", task.canonical_task_id));
        }
    }
    defects.into_iter().collect()
}

fn matches_tool(declared: &str, qualified: &str) -> bool {
    if declared.starts_with("mcp__") { declared == qualified }
    else { raw_tool_name(declared) == raw_tool_name(qualified) }
}

pub(super) fn set_findings(project: &Path, root: &Path) -> Vec<GateFinding> {
    let mut findings = Vec::new();
    // Unreadable task files are handled by the shared operational preflight.
    for path in task_files_under(root).unwrap_or_default() {
        let Ok(raw) = std::fs::read_to_string(&path) else { continue; };
        let Ok(task) = parse_task_file(&path, &raw) else { continue; };
        findings.extend(inspect(project, &task, &raw).into_iter().map(|text| {
            GateFinding::new(GateId::WorkflowLintTaskSet, text, &task.canonical_task_id,
                Some(path.clone()), archon_workflow::RemediationScope::Body)
        }));
    }
    findings
}
