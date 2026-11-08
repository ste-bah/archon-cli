use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::command::workflow_gate::{GateFinding, GateId};
use anyhow::{Result, anyhow};
use archon_workflow::RemediationScope;
use archon_workflow::defect::DeterministicDefect;
use archon_workflow::prd_tools::{
    PrdToolDefect, actionable_tokens, complete_mcp_tool, derived_server_links,
    prd_tool_declaration, prd_tool_defects, tool_matches,
};
use archon_workflow::task_set_contract::TASK_SKELETON_FILE;
use archon_workflow::task_universe::{
    WorkflowV2TaskUniverseTask, parsing::parse_task_file, task_files_under,
};

type Task = (PathBuf, WorkflowV2TaskUniverseTask);

pub(super) fn set_findings(project: &Path, root: &Path) -> Result<Vec<GateFinding>> {
    let (claims, _) = crate::command::topology_task_graph::task_requirement_claims_tolerant(root)?;
    let Ok(Some(prd_path)) = super::coverage::resolve_prd(root, &claims) else {
        return Ok(vec![]);
    };
    let Ok(prd) = std::fs::read_to_string(&prd_path) else {
        return Ok(vec![]);
    };
    let (config, permitted) = crate::command::workflow_mcp::project_permitted_tools(project)
        .map_err(|error| anyhow!(error))?;
    let message_prd = prd_path.strip_prefix(project).unwrap_or(&prd_path);
    let message_config = config.strip_prefix(project).unwrap_or(&config);
    let declaration = prd_tool_declaration(&prd);
    let actionable = actionable_tokens(&prd);
    let derived = derived_server_links(&prd, &permitted);
    let mut findings = Vec::new();
    for defect in prd_tool_defects(&declaration, &permitted) {
        match &defect {
            PrdToolDefect::NotPermitted { token } if actionable.contains(token) => {
                findings.push(
                    GateFinding::new(
                        GateId::WorkflowLintTaskSet,
                        defect.message(message_prd, message_config),
                        token,
                        Some(prd_path.clone()),
                        RemediationScope::PrdInput,
                    )
                    .with_defect(DeterministicDefect::new(
                        defect.code(),
                        token,
                        "prd/required_tools",
                    )),
                );
            }
            PrdToolDefect::NotPermitted { .. } | PrdToolDefect::Unlinked { .. } => {
                eprintln!("prd tools: {}", defect.message(message_prd, message_config));
            }
            PrdToolDefect::Undeclared { .. } => {}
        }
    }
    if declaration.section_line.is_none()
        && declaration.tokens.is_empty()
        && derived.values().all(BTreeSet::is_empty)
    {
        eprintln!(
            "prd tools: no Required Tools section; {} permitted; no PRD id names a server",
            permitted.len()
        );
    }
    let mut tasks = Vec::new();
    let mut skipped = 0;
    for path in task_files_under(root)? {
        let raw = std::fs::read_to_string(&path)?;
        match parse_task_file(&path, &raw) {
            Ok(task) => tasks.push((path, task)),
            Err(_) => skipped += 1,
        }
    }
    if skipped > 0 {
        eprintln!("prd tools: union not judged: {skipped} task file(s) did not parse");
        return Ok(findings);
    }
    let skeleton = std::fs::read(root.join(TASK_SKELETON_FILE))
        .ok()
        .and_then(|bytes| {
            serde_json::from_slice::<archon_workflow::task_skeleton::TaskSkeleton>(&bytes).ok()
        });
    tasks.sort_by(|(_, left), (_, right)| {
        let rank = |id: &str| {
            skeleton
                .as_ref()
                .and_then(|s| s.tasks.iter().position(|task| task.task_id == id))
                .unwrap_or(usize::MAX)
        };
        (rank(&left.canonical_task_id), &left.canonical_task_id)
            .cmp(&(rank(&right.canonical_task_id), &right.canonical_task_id))
    });
    for (token, links) in &declaration.tokens {
        if !actionable.contains(token) || !permitted.contains(token) {
            continue;
        }
        let server = complete_mcp_tool(token).and_then(|(server, _)| server);
        let links = if links.is_empty() {
            server
                .and_then(|server| derived.get(server))
                .unwrap_or(links)
        } else {
            links
        };
        if links.is_empty() {
            continue;
        }
        if tasks.iter().any(|(_, task)| {
            task.required_tools
                .iter()
                .any(|entry| tool_matches(entry, token))
        }) {
            continue;
        }
        findings.push(unclaimed(
            root,
            &tasks,
            links,
            std::slice::from_ref(token),
            true,
        ));
    }
    if declaration.section_line.is_none() {
        for (server, links) in derived {
            let candidates: Vec<_> = permitted
                .iter()
                .filter(|token| {
                    complete_mcp_tool(token).is_some_and(|(name, _)| name == Some(server.as_str()))
                })
                .cloned()
                .collect();
            if links.is_empty() {
                continue;
            }
            let covered = tasks.iter().any(|(_, task)| {
                task.implements.iter().any(|id| links.contains(id))
                    && task
                        .required_tools
                        .iter()
                        .any(|entry| candidates.iter().any(|token| tool_matches(entry, token)))
            });
            if !covered {
                findings.push(unclaimed(root, &tasks, &links, &candidates, false));
            }
        }
    }
    let mut merged: Vec<GateFinding> = Vec::new();
    for finding in findings {
        if let Some(existing) = merged
            .iter_mut()
            .find(|existing| existing.deterministic_defect == finding.deterministic_defect)
        {
            existing.text.push_str(" | ");
            existing.text.push_str(&finding.text);
        } else {
            merged.push(finding);
        }
    }
    Ok(merged)
}

fn unclaimed(
    root: &Path,
    tasks: &[Task],
    links: &BTreeSet<String>,
    candidates: &[String],
    declared: bool,
) -> GateFinding {
    let owner = if declared {
        tasks
            .iter()
            .find(|(_, task)| task.implements.iter().any(|id| links.contains(id)))
    } else {
        tasks
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                let count = entry
                    .1
                    .implements
                    .iter()
                    .filter(|id| links.contains(*id))
                    .collect::<BTreeSet<_>>()
                    .len();
                (count > 0).then_some(((std::cmp::Reverse(count), index), entry))
            })
            .min_by_key(|(rank, _)| *rank)
            .map(|(_, entry)| entry)
    };
    let names = candidates.join(", ");
    let (subject, path, scope, code, field, text) = if let Some((path, task)) = owner {
        let id = &task.canonical_task_id;
        let owned = task
            .implements
            .iter()
            .filter(|id| links.contains(*id))
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        let text = if declared {
            format!(
                "task {id}: required_tools lacks MCP tool '{names}', which the PRD declares for {owned} that this task implements; add exactly '{names}' to required_tools (the run will require this task to call it)"
            )
        } else {
            format!(
                "task {id}: PRD ids {owned} name a configured server; required_tools must include one or more of its permitted tools: {names} (the run will require this task to call each declared tool)"
            )
        };
        (
            id.clone(),
            path.clone(),
            RemediationScope::Body,
            "prd_tool_unclaimed",
            "required_tools",
            text,
        )
    } else {
        let id = links.first().expect("nonempty links").clone();
        let links = links.iter().cloned().collect::<Vec<_>>().join(", ");
        let text = if declared {
            format!(
                "PRD MCP tool '{names}' is declared for {links}, but no task implements any of them; add one of these ids to the implements of the task whose work calls '{names}'"
            )
        } else {
            format!(
                "PRD ids {links} name a configured server, but no task implements any of them; add one of these ids to the implements of the task whose work calls one or more of its permitted tools: {names}"
            )
        };
        (
            id,
            root.join(TASK_SKELETON_FILE),
            RemediationScope::Skeleton,
            "prd_tool_link_unclaimed",
            "implements",
            text,
        )
    };
    GateFinding::new(
        GateId::WorkflowLintTaskSet,
        text,
        &subject,
        Some(path),
        scope,
    )
    .with_defect(DeterministicDefect::new(code, subject, field))
}
