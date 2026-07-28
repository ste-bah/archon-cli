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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct WorkflowV2TaskUniverseTask {
    pub(super) canonical_task_id: String,
    #[serde(default)]
    pub(super) aliases: Vec<String>,
    pub(super) source_path: String,
    #[serde(default)]
    pub(super) dependency_ids: Vec<String>,
    #[serde(default)]
    pub(super) title: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) acceptance_criteria: Vec<String>,
    /// Explicit artifact declarations from the task file (paths relative to
    /// the project artifact root). Part of the declared artifact contract.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) artifact_requirements: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) required_env_keys: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) required_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) deliverable_contracts: Vec<WorkflowV2DeliverableContract>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(super) struct WorkflowV2DeliverableContract {
    pub(super) kind: String,
    pub(super) artifact_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) typed_verifier_command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) registry_path: Option<String>,
    #[serde(default)]
    pub(super) required_universe: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) data_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) universe_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) cells_field: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) cell_identity_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) required_true_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) required_nonempty_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) positive_count_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) gaps_field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) registry_records_field: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) registry_key_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) registry_required_true_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) registry_status_field: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) registry_allowed_statuses: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) registry_count_field: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(super) registry_identity_fields: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) payload_path_field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) payload_format: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) required_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) non_constant_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) series_value_fields: Vec<String>,
    #[serde(default)]
    pub(super) series_overlap_min_rows: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) request_path_field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) requested_count_field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) response_path_field: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(super) response_identity_fields: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) validation_path_field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) validation_status_field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) validation_checks_field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) validation_check_status_field: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) validation_failed_values: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) validation_passed_values: Vec<String>,
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
            let mut parsed = parse_task_file(&path, &raw)?;
            merge_project_capabilities(&mut parsed, &path)?;
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
    text.contains("decomposed prd") || text.contains("task-*.md") || text.contains("/tasks/prd-")
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

#[path = "workflow_live_task_universe_parsing.rs"]
mod parsing;
use parsing::{merge_project_capabilities, parse_task_file};

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
