use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use archon_workflow::{WorkflowError, WorkflowResult};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct WorkflowV2TaskUniverse {
    pub(super) schema_version: String,
    pub(super) source_roots: Vec<String>,
    pub(super) tasks: Vec<WorkflowV2TaskUniverseTask>,
}

impl WorkflowV2TaskUniverse {
    #[cfg(test)]
    pub(super) fn canonical_ids(&self) -> Vec<String> {
        self.tasks
            .iter()
            .map(|task| task.canonical_task_id.clone())
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct WorkflowV2TaskUniverseTask {
    pub(super) canonical_task_id: String,
    #[serde(default)]
    pub(super) aliases: Vec<String>,
    pub(super) source_path: String,
    #[serde(default)]
    pub(super) dependency_ids: Vec<String>,
    #[serde(default)]
    pub(super) title: Option<String>,
}

pub(super) fn extract_task_universe_for_generated_run(
    task: &str,
) -> WorkflowResult<Option<WorkflowV2TaskUniverse>> {
    if !requires_authoritative_task_universe(task) {
        return Ok(None);
    }
    let roots = task_roots_from_text(task);
    if roots.is_empty() {
        return Err(WorkflowError::SpecInvalid(
            "generated decomposed PRD workflow requires local TASK-*.md evidence before planning"
                .to_string(),
        ));
    }
    let mut raw_tasks = BTreeMap::new();
    let mut referenced_task_ids = BTreeSet::new();
    let mut source_roots = BTreeSet::new();
    for path in markdown_evidence_paths_from_text(task) {
        if !path.exists()
            || path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(is_task_markdown_name)
        {
            continue;
        }
        source_roots.insert(path.display().to_string());
        let raw = fs::read_to_string(&path).map_err(|err| WorkflowError::Io {
            path: path.clone(),
            source: err,
        })?;
        for task_id in task_ids_from_text(&raw) {
            referenced_task_ids.insert(task_id);
        }
    }
    for root in roots {
        source_roots.insert(root.display().to_string());
        for path in task_files_under(&root)? {
            let raw = fs::read_to_string(&path).map_err(|err| WorkflowError::Io {
                path: path.clone(),
                source: err,
            })?;
            let parsed = parse_task_file(&path, &raw)?;
            raw_tasks.insert(parsed.canonical_task_id.clone(), parsed);
        }
    }
    if raw_tasks.is_empty() {
        return Err(WorkflowError::SpecInvalid(
            "generated decomposed PRD workflow found no parseable TASK-*.md files".to_string(),
        ));
    }

    let canonical_ids = raw_tasks.keys().cloned().collect::<BTreeSet<_>>();
    for referenced in referenced_task_ids {
        if !canonical_ids.contains(&referenced) {
            return Err(WorkflowError::SpecInvalid(format!(
                "generated decomposed PRD workflow references {referenced} but no matching TASK-*.md file was found"
            )));
        }
    }

    let mut aliases = BTreeMap::new();
    for task_id in &canonical_ids {
        aliases.insert(task_id.clone(), task_id.clone());
        if let Some(short) = short_task_alias(task_id) {
            aliases.insert(short, task_id.clone());
        }
    }

    let mut tasks = Vec::new();
    for mut task in raw_tasks.into_values() {
        task.aliases = sorted_unique(
            std::iter::once(short_task_alias(&task.canonical_task_id))
                .flatten()
                .collect(),
        );
        let mut normalized_dependencies = Vec::new();
        for dep in &task.dependency_ids {
            let Some(canonical) = aliases.get(dep).cloned() else {
                return Err(WorkflowError::SpecInvalid(format!(
                    "generated decomposed PRD workflow has unresolved dependency reference '{dep}' in {}",
                    task.source_path
                )));
            };
            normalized_dependencies.push(canonical);
        }
        task.dependency_ids = sorted_unique(normalized_dependencies);
        tasks.push(task);
    }
    tasks.sort_by(|left, right| left.canonical_task_id.cmp(&right.canonical_task_id));
    validate_task_dependency_graph(&tasks)?;

    Ok(Some(WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: source_roots.into_iter().collect(),
        tasks,
    }))
}

fn requires_authoritative_task_universe(task: &str) -> bool {
    let text = task.to_ascii_lowercase();
    text.contains("decomposed prd")
        || text.contains("task-tdl")
        || text.contains("task-*.md")
        || text.contains("/tasks/prd-")
}

fn task_roots_from_text(text: &str) -> Vec<PathBuf> {
    let mut roots = BTreeSet::new();
    for path in absolute_paths_from_text(text) {
        if path.is_dir() {
            if directory_has_task_files(&path) {
                roots.insert(path);
            }
            continue;
        }
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(is_task_markdown_name)
        {
            if let Some(parent) = path.parent() {
                roots.insert(parent.to_path_buf());
            }
            continue;
        }
        if let Some(parent) = path.parent() {
            if directory_has_task_files(parent) {
                roots.insert(parent.to_path_buf());
            } else if let Some(grandparent) = parent.parent()
                && directory_has_task_files(grandparent)
            {
                roots.insert(grandparent.to_path_buf());
            }
        }
    }
    roots.into_iter().collect()
}

fn directory_has_task_files(path: &Path) -> bool {
    fs::read_dir(path)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .any(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(is_task_markdown_name)
        })
}

fn task_files_under(root: &Path) -> WorkflowResult<Vec<PathBuf>> {
    let entries = fs::read_dir(root).map_err(|err| WorkflowError::Io {
        path: root.to_path_buf(),
        source: err,
    })?;
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| WorkflowError::Io {
            path: root.to_path_buf(),
            source: err,
        })?;
        let path = entry.path();
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(is_task_markdown_name)
        {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

fn is_task_markdown_name(name: &str) -> bool {
    name.starts_with("TASK-") && name.ends_with(".md")
}

fn parse_task_file(path: &Path, raw: &str) -> WorkflowResult<WorkflowV2TaskUniverseTask> {
    let path_task_id = task_id_from_task_path(path).ok_or_else(|| {
        WorkflowError::SpecInvalid(format!(
            "generated decomposed PRD workflow could not parse canonical task_id from {}",
            path.display()
        ))
    })?;
    let canonical_task_id = match first_field(raw, "task_id") {
        Some(field_task_id) => {
            let parsed = canonical_task_id_from_ref(&field_task_id).ok_or_else(|| {
                WorkflowError::SpecInvalid(format!(
                    "generated decomposed PRD workflow has invalid task_id '{field_task_id}' in {}",
                    path.display()
                ))
            })?;
            if parsed != path_task_id {
                return Err(WorkflowError::SpecInvalid(format!(
                    "generated decomposed PRD workflow task_id '{parsed}' does not match filename task id '{path_task_id}' in {}",
                    path.display()
                )));
            }
            parsed
        }
        None => path_task_id,
    };
    let dependency_ids = sorted_unique(first_list_field(raw, "depends_on"));
    Ok(WorkflowV2TaskUniverseTask {
        canonical_task_id,
        aliases: Vec::new(),
        source_path: path.display().to_string(),
        dependency_ids,
        title: raw
            .lines()
            .find_map(|line| line.trim().strip_prefix("# "))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
    })
}

fn first_field(raw: &str, field: &str) -> Option<String> {
    let prefix = format!("{field}:");
    raw.lines().find_map(|line| {
        line.trim()
            .strip_prefix(&prefix)
            .map(str::trim)
            .map(trim_yaml_scalar)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

fn first_list_field(raw: &str, field: &str) -> Vec<String> {
    let prefix = format!("{field}:");
    let Some(value) = raw
        .lines()
        .find_map(|line| line.trim().strip_prefix(&prefix).map(str::trim))
    else {
        return Vec::new();
    };
    let value = value
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim();
    if value.is_empty() {
        return Vec::new();
    }
    value
        .split(',')
        .map(trim_yaml_scalar)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

fn trim_yaml_scalar(value: &str) -> &str {
    value.trim().trim_matches(|ch| matches!(ch, '"' | '\''))
}

fn absolute_paths_from_text(text: &str) -> Vec<PathBuf> {
    text.split_whitespace()
        .map(|part| {
            part.trim_matches(|ch: char| {
                matches!(
                    ch,
                    '"' | '\'' | '`' | ',' | ';' | ':' | ')' | '(' | '[' | ']' | '<' | '>'
                )
            })
        })
        .filter(|part| part.starts_with('/'))
        .map(PathBuf::from)
        .collect()
}

fn markdown_evidence_paths_from_text(text: &str) -> Vec<PathBuf> {
    absolute_paths_from_text(text)
        .into_iter()
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("md"))
        .collect()
}

fn task_ids_from_text(raw: &str) -> Vec<String> {
    sorted_unique(
        raw.split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '-'))
            .filter_map(canonical_task_id_from_ref)
            .collect(),
    )
}

fn canonical_task_id_from_ref(value: &str) -> Option<String> {
    let parts = value.split('-').collect::<Vec<_>>();
    if parts.len() != 3 {
        return None;
    }
    let first = parts[0];
    let second = parts[1];
    let third = parts[2];
    if first != "TASK"
        || second.is_empty()
        || !second
            .chars()
            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit())
        || third.len() != 3
        || !third.chars().all(|ch| ch.is_ascii_digit())
    {
        return None;
    }
    Some(format!("{first}-{second}-{third}"))
}

fn task_id_from_task_path(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    canonical_task_id_from_stem(stem)
}

fn canonical_task_id_from_stem(stem: &str) -> Option<String> {
    let mut parts = stem.split('-');
    let first = parts.next()?;
    let second = parts.next()?;
    let third = parts.next()?;
    if first != "TASK"
        || second.is_empty()
        || !second
            .chars()
            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit())
        || third.len() != 3
        || !third.chars().all(|ch| ch.is_ascii_digit())
    {
        return None;
    }
    Some(format!("{first}-{second}-{third}"))
}

fn short_task_alias(canonical: &str) -> Option<String> {
    let digits = canonical.rsplit('-').next()?;
    (!digits.is_empty() && digits.chars().all(|ch| ch.is_ascii_digit()))
        .then(|| format!("T{digits}"))
}

fn sorted_unique(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn validate_task_dependency_graph(tasks: &[WorkflowV2TaskUniverseTask]) -> WorkflowResult<()> {
    let graph = tasks
        .iter()
        .map(|task| {
            (
                task.canonical_task_id.clone(),
                task.dependency_ids.clone().into_iter().collect::<Vec<_>>(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut state = BTreeMap::<String, VisitState>::new();
    for task_id in graph.keys() {
        visit_dependency_node(task_id, &graph, &mut state, &mut Vec::new())?;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VisitState {
    Visiting,
    Done,
}

fn visit_dependency_node(
    task_id: &str,
    graph: &BTreeMap<String, Vec<String>>,
    state: &mut BTreeMap<String, VisitState>,
    stack: &mut Vec<String>,
) -> WorkflowResult<()> {
    match state.get(task_id).copied() {
        Some(VisitState::Done) => return Ok(()),
        Some(VisitState::Visiting) => {
            stack.push(task_id.to_string());
            return Err(WorkflowError::SpecInvalid(format!(
                "generated decomposed PRD workflow task dependency cycle detected: {}",
                stack.join(" -> ")
            )));
        }
        None => {}
    }
    state.insert(task_id.to_string(), VisitState::Visiting);
    stack.push(task_id.to_string());
    for dependency in graph.get(task_id).into_iter().flatten() {
        visit_dependency_node(dependency, graph, state, stack)?;
    }
    stack.pop();
    state.insert(task_id.to_string(), VisitState::Done);
    Ok(())
}

#[cfg(test)]
#[path = "workflow_live_task_universe_tests.rs"]
mod tests;
