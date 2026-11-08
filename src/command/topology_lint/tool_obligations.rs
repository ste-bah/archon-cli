//! Task-local MCP obligations checked before body publication and at set gates.
//! HTTP/service adapters are not MCP clients; no ambient tool union is granted.

use crate::command::workflow_gate::{GateFinding, GateId};
use archon_workflow::prd_tools::{complete_mcp_tool, tool_matches};
use archon_workflow::task_universe::{
    WorkflowV2TaskUniverseTask, parsing::parse_task_file, task_files_under,
};
use std::collections::BTreeSet;
use std::path::Path;

pub(super) fn inspect(project: &Path, task: &WorkflowV2TaskUniverseTask, raw: &str) -> Vec<String> {
    let (config, permitted) = match crate::command::workflow_mcp::project_permitted_tools(project) {
        Ok(input) => input,
        Err(error) => {
            let prefix = format!("{}{}", project.display(), std::path::MAIN_SEPARATOR);
            return vec![error.replace(&prefix, "")];
        }
    };
    let config = config.strip_prefix(project).unwrap_or(&config).display();
    let granted: BTreeSet<_> = permitted
        .iter()
        .filter(|name| {
            task.required_tools
                .iter()
                .any(|tool| tool_matches(tool, name))
        })
        .cloned()
        .collect();
    let mut defects = BTreeSet::new();
    for contract in &task.deliverable_contracts {
        let kind = contract.kind.to_ascii_lowercase();
        let parts: Vec<_> = kind.split(|c: char| !c.is_alphanumeric()).collect();
        if !parts.contains(&"mcp") {
            continue;
        }
        let server = parts
            .iter()
            .position(|part| *part == "mcp")
            .filter(|index| *index > 0)
            .map(|index| parts[..index].join("-"));
        let candidates: Vec<_> = permitted
            .iter()
            .filter(|name| {
                server
                    .as_ref()
                    .is_none_or(|server| name.starts_with(&format!("mcp__{server}__")))
            })
            .collect();
        if !candidates.iter().any(|name| granted.contains(*name)) {
            defects.insert(format!(
                "{}: required_tools does not grant the MCP access declared by contract '{}'; declare the exact project-permitted MCP tools this task must exercise, using {config}. Available matching tools: {}. If this is an HTTP or offline adapter, correct its contract rather than declaring unrelated tools.",
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
        if !focused {
            continue;
        }
        for token in
            line.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == ':' || c == '-'))
        {
            let token = token.trim_end_matches([':', '-', '_', '.']);
            let named = permitted.iter().find(|name| tool_matches(token, name));
            let invocation = complete_mcp_tool(token);
            if (invocation.is_some() || named.is_some())
                && !named.is_some_and(|name| granted.contains(name))
            {
                let server_tools: Vec<_> = invocation
                    .and_then(|(server, _)| server)
                    .map(|server| {
                        let prefix = format!("mcp__{server}__");
                        permitted
                            .iter()
                            .filter(|name| name.starts_with(&prefix))
                            .map(String::as_str)
                            .collect()
                    })
                    .unwrap_or_default();
                let hint = if server_tools.is_empty() {
                    String::new()
                } else {
                    format!("; permitted for that server: {}", server_tools.join(", "))
                };
                defects.insert(format!("{}: required_tools is missing a permitted grant for focused MCP call '{token}'; use an exact permitted name from {config} and declare that invocation{hint}", task.canonical_task_id));
            }
        }
    }
    for tool in task
        .required_tools
        .iter()
        .filter(|tool| tool.starts_with("mcp__") || tool.starts_with("mcp_action:"))
    {
        if !permitted.iter().any(|name| tool_matches(tool, name)) {
            defects.insert(format!("{}: required_tools names MCP tool '{tool}' which is not permitted by the project config {config}", task.canonical_task_id));
        }
    }
    defects.into_iter().collect()
}

pub(super) fn set_findings(project: &Path, root: &Path) -> Vec<GateFinding> {
    let mut findings = Vec::new();
    // Unreadable task files are handled by the shared operational preflight.
    for path in task_files_under(root).unwrap_or_default() {
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(task) = parse_task_file(&path, &raw) else {
            continue;
        };
        findings.extend(inspect(project, &task, &raw).into_iter().map(|text| {
            GateFinding::new(
                GateId::WorkflowLintTaskSet,
                text,
                &task.canonical_task_id,
                Some(path.clone()),
                archon_workflow::RemediationScope::Body,
            )
        }));
    }
    findings
}
