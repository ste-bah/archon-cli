use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use archon_workflow::{WorkflowError, WorkflowResult};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct WorkflowV2TaskUniverse {
    pub(crate) schema_version: String,
    pub(crate) source_roots: Vec<String>,
    pub(crate) tasks: Vec<WorkflowV2TaskUniverseTask>,
}

impl WorkflowV2TaskUniverse {
    pub(crate) fn resolve_canonical_task_id(&self, input: &str) -> WorkflowResult<String> {
        let wanted = normalized_task_alias(input);
        if wanted.is_empty() {
            return Err(WorkflowError::SpecInvalid(
                "restart task id must not be empty".to_string(),
            ));
        }
        let mut matches = Vec::new();
        for task in &self.tasks {
            if task
                .aliases_for_resolution()
                .into_iter()
                .any(|alias| normalized_task_alias(&alias) == wanted)
            {
                matches.push(task.canonical_task_id.clone());
            }
        }
        matches.sort();
        matches.dedup();
        match matches.as_slice() {
            [task_id] => Ok(task_id.clone()),
            [] => Err(WorkflowError::SpecInvalid(format!(
                "task '{input}' did not match any canonical task in the generated task universe"
            ))),
            many => Err(WorkflowError::SpecInvalid(format!(
                "task alias '{input}' is ambiguous across canonical tasks {}; use the full canonical task ID",
                many.join(", ")
            ))),
        }
    }

    pub(crate) fn downstream_task_closure(&self, canonical_task_id: &str) -> BTreeSet<String> {
        let mut affected = BTreeSet::from([canonical_task_id.to_string()]);
        let mut changed = true;
        while changed {
            changed = false;
            for task in &self.tasks {
                if affected.contains(&task.canonical_task_id) {
                    continue;
                }
                if task
                    .dependency_ids
                    .iter()
                    .any(|dependency| affected.contains(dependency))
                {
                    affected.insert(task.canonical_task_id.clone());
                    changed = true;
                }
            }
        }
        affected
    }

    #[cfg(test)]
    pub(super) fn canonical_ids(&self) -> Vec<String> {
        self.tasks
            .iter()
            .map(|task| task.canonical_task_id.clone())
            .collect()
    }
}

impl WorkflowV2TaskUniverseTask {
    fn aliases_for_resolution(&self) -> Vec<String> {
        let mut aliases = vec![self.canonical_task_id.clone()];
        if let Some(without_prefix) = self.canonical_task_id.strip_prefix("TASK-") {
            aliases.push(without_prefix.to_string());
        }
        aliases.extend(self.aliases.iter().cloned());
        aliases
    }
}

fn normalized_task_alias(value: &str) -> String {
    value
        .trim()
        .to_ascii_uppercase()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect()
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct WorkflowV2TaskUniverseTask {
    pub(crate) canonical_task_id: String,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) instance_source_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) instance_source_records_field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) instance_artifact_field: Option<String>,
    #[serde(default)]
    pub(super) min_instances: usize,
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
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(super) minimum_count_fields: BTreeMap<String, u64>,
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
    #[serde(default)]
    pub(super) registry_minimum_count: u64,
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
    /// Name of the payload field holding each record's observation instant. When
    /// declared, the verifier rejects any record dated after the verification
    /// time: an observed series cannot contain future records, and fabricated
    /// payloads built by incrementing a timestamp N times routinely overshoot
    /// now. Domain-neutral — the contract names the field, the engine only
    /// compares instants.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) observed_time_field: Option<String>,
    /// Weekday indices (Mon=0 … Sun=6) on which the observed venue does not
    /// trade, and specific non-trading dates. When either is declared the
    /// verifier rejects records dated to a closed session. This is external
    /// truth rather than a threshold: an evenly spaced generated series lands on
    /// closed days by construction, and there is no number to fabricate toward.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) closed_weekdays: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) closed_dates: Vec<String>,
    /// Overrides for the synthetic-series step-variety check, the minimum
    /// percentage of first differences that must be distinct. Left unset in task
    /// specs by design — a declared numeric threshold is a target to fabricate
    /// against, so the defaults live in the engine where the agent cannot read
    /// them. Integer percent rather than a float ratio because this struct
    /// derives Ord and f64 does not implement it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) step_variety_min_rows: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) step_variety_min_percent: Option<u32>,
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

fn merge_project_capabilities(
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
