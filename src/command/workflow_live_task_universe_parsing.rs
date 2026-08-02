use std::fs;
use std::path::Path;

use archon_workflow::{WorkflowError, WorkflowResult};

use super::{
    WorkflowV2TaskUniverseTask, canonical_task_id_from_ref, sorted_unique, task_id_from_task_path,
};

pub(super) fn parse_task_file(
    path: &Path,
    raw: &str,
) -> WorkflowResult<WorkflowV2TaskUniverseTask> {
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
    let metadata = task_metadata(raw);
    let dependency_ids = sorted_unique({
        let declared = metadata_strings(&metadata, "depends_on");
        if declared.is_empty() {
            first_list_field(raw, "depends_on")
        } else {
            declared
        }
    });
    let deliverable_contracts = match metadata.get("deliverable_contracts") {
        Some(value) => serde_json::from_value(value.clone())?,
        None => Vec::new(),
    };
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
        acceptance_criteria: declared_task_section_items(raw, "acceptance criteria"),
        // Additive: the per-task adversarial reviewer reads these verbatim.
        adversarial_review_notes: declared_task_section_items(raw, "adversarial review notes"),
        artifact_requirements: declared_task_artifact_requirements(raw),
        required_env_keys: sorted_unique(metadata_strings(&metadata, "required_env_keys")),
        required_tools: sorted_unique(metadata_strings(&metadata, "required_tools")),
        deliverable_contracts,
    })
}

fn task_metadata(raw: &str) -> serde_json::Value {
    let mut yaml = String::new();
    let mut in_yaml = false;
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed == "```yaml" || trimmed == "```yml" || trimmed == "---" && yaml.is_empty() {
            in_yaml = true;
            continue;
        }
        if in_yaml && (trimmed == "```" || trimmed == "---") {
            break;
        }
        if in_yaml {
            yaml.push_str(line);
            yaml.push('\n');
        }
    }
    serde_yaml_ng::from_str(&yaml).unwrap_or_else(|_| serde_json::json!({}))
}

fn metadata_strings(metadata: &serde_json::Value, field: &str) -> Vec<String> {
    match metadata.get(field) {
        Some(serde_json::Value::String(value)) => vec![value.clone()],
        Some(serde_json::Value::Array(values)) => values
            .iter()
            .filter_map(serde_json::Value::as_str)
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

pub(super) fn merge_project_capabilities(
    task: &mut WorkflowV2TaskUniverseTask,
    task_path: &Path,
) -> WorkflowResult<()> {
    let Some(project_root) = task_path
        .ancestors()
        .find(|ancestor| ancestor.join(".archon").is_dir())
    else {
        return Ok(());
    };
    let manifest_path = project_root.join(".archon/project.json");
    if !manifest_path.is_file() {
        return Ok(());
    }
    let raw = fs::read_to_string(&manifest_path).map_err(|source| WorkflowError::Io {
        path: manifest_path.clone(),
        source,
    })?;
    let manifest: serde_json::Value = serde_json::from_str(&raw)?;
    task.required_env_keys = sorted_unique(
        task.required_env_keys
            .iter()
            .cloned()
            .chain(metadata_strings(&manifest, "required_env_keys"))
            .collect(),
    );
    let mut project_tools = metadata_strings(&manifest, "required_tools");
    if let Some(bundles) = manifest
        .get("tool_bundles")
        .and_then(serde_json::Value::as_object)
    {
        for tools in bundles.values() {
            project_tools.extend(match tools {
                serde_json::Value::Array(values) => values
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::to_string)
                    .collect(),
                _ => Vec::new(),
            });
        }
    }
    task.required_tools = sorted_unique(
        task.required_tools
            .iter()
            .cloned()
            .chain(project_tools)
            .collect(),
    );
    Ok(())
}

/// Explicit artifact declarations in a TASK-*.md file: list items under an
/// "Artifact Requirements" heading, plus `artifact_requirements:` field lists.
/// These declarations flow into every implementation/remediation item for the
/// task, so the agent that owns the task is always instructed to produce them.
fn declared_task_artifact_requirements(raw: &str) -> Vec<String> {
    let mut requirements = first_list_field(raw, "artifact_requirements");
    let mut in_section = false;
    for line in raw.lines() {
        let trimmed = line.trim();
        if let Some(heading) = trimmed.strip_prefix('#') {
            in_section = heading
                .trim_start_matches('#')
                .trim()
                .eq_ignore_ascii_case("artifact requirements");
            continue;
        }
        if !in_section {
            continue;
        }
        if let Some(item) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            let path = item.trim().trim_matches('`').trim();
            if !path.is_empty() {
                requirements.push(path.to_string());
            }
        }
    }
    sorted_unique(requirements)
}

fn declared_task_section_items(raw: &str, section: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut in_section = false;
    for line in raw.lines() {
        let trimmed = line.trim();
        if let Some(heading) = trimmed.strip_prefix('#') {
            in_section = heading
                .trim_start_matches('#')
                .trim()
                .eq_ignore_ascii_case(section);
            continue;
        }
        if !in_section {
            continue;
        }
        if let Some(item) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            let item = item.trim();
            if !item.is_empty() {
                items.push(item.to_string());
            }
        }
    }
    sorted_unique(items)
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
