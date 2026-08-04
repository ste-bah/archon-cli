use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowV2TaskUniverse {
    pub schema_version: String,
    pub source_roots: Vec<String>,
    pub tasks: Vec<WorkflowV2TaskUniverseTask>,
}

impl WorkflowV2TaskUniverse {
    pub fn resolve_canonical_task_id(&self, input: &str) -> WorkflowResult<String> {
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

    pub fn downstream_task_closure(&self, canonical_task_id: &str) -> BTreeSet<String> {
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
pub struct WorkflowV2TaskUniverseTask {
    pub canonical_task_id: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub source_path: String,
    #[serde(default)]
    pub dependency_ids: Vec<String>,
    /// The task file's declared `blocks:` — the reverse edge of `depends_on`.
    ///
    /// Kept verbatim as declared rather than folded away, so the reconciled
    /// graph can be checked against what each file actually said. `blocks` used
    /// to be parsed by nothing at all: a task file that expressed its ordering
    /// only in that direction contributed no edge and its dependents ran early.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocks_ids: Vec<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub complexity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Declared `implements:` and `shared_append_target_files:` — empty only
    /// because the author declared them so; see `parsing` for why absent differs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub implements: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shared_append_target_files: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub acceptance_criteria: Vec<String>,
    /// Per-task adversarial checks declared under an `## Adversarial Review
    /// Notes` heading in the TASK file. Consumed ONLY by the per-task
    /// `adversarial-review` stage: they are the task author's own falsification
    /// hypotheses, and they are meaningless to a reviewer holding every task at
    /// once, which is why they were unreachable while review was a single
    /// terminal reduce.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub adversarial_review_notes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files_expected_to_change: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files_forbidden_to_change: Vec<String>,
    /// Explicit artifact declarations from the task file (paths relative to
    /// the project artifact root). Part of the declared artifact contract.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifact_requirements: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_env_keys: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deliverable_contracts: Vec<WorkflowV2DeliverableContract>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct WorkflowV2DeliverableContract {
    pub kind: String,
    pub artifact_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub typed_verifier_command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_source_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_source_records_field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_artifact_field: Option<String>,
    #[serde(default)]
    pub min_instances: usize,
    #[serde(default)]
    pub required_universe: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub universe_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cells_field: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cell_identity_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_true_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_nonempty_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub positive_count_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub minimum_count_fields: BTreeMap<String, u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gaps_field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry_records_field: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub registry_key_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub registry_required_true_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry_status_field: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub registry_allowed_statuses: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry_count_field: Option<String>,
    #[serde(default)]
    pub registry_minimum_count: u64,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub registry_identity_fields: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload_path_field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload_format: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub non_constant_fields: Vec<String>,
    /// Name of the payload field holding each record's observation instant. When
    /// declared, the verifier rejects any record dated after the verification
    /// time: an observed series cannot contain future records, and fabricated
    /// payloads built by incrementing a timestamp N times routinely overshoot
    /// now. Domain-neutral — the contract names the field, the engine only
    /// compares instants.
    /// Format of the declared deliverable: `json` (default) or `text`.
    ///
    /// A contract may legitimately declare a prose or tabular artifact — an
    /// inventory, a written report. Parsing one as JSON fails on line 1 however
    /// good the work is, and a fail-closed gate then demotes it permanently
    /// because no remediation can make markdown parse. A textual deliverable is
    /// checked for existence and non-emptiness, which is all a contract can
    /// honestly assert about unstructured content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_time_field: Option<String>,
    /// Weekday indices (Mon=0 … Sun=6) on which the observed venue does not
    /// trade, and specific non-trading dates. When either is declared the
    /// verifier rejects records dated to a closed session. This is external
    /// truth rather than a threshold: an evenly spaced generated series lands on
    /// closed days by construction, and there is no number to fabricate toward.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_weekdays: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub closed_dates: Vec<String>,
    /// Overrides for the synthetic-series step-variety check, the minimum
    /// percentage of first differences that must be distinct. Left unset in task
    /// specs by design — a declared numeric threshold is a target to fabricate
    /// against, so the defaults live in the engine where the agent cannot read
    /// them. Integer percent rather than a float ratio because this struct
    /// derives Ord and f64 does not implement it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_variety_min_rows: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_variety_min_percent: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub series_value_fields: Vec<String>,
    #[serde(default)]
    pub series_overlap_min_rows: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_path_field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_count_field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_path_field: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub response_identity_fields: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_path_field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_status_field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_checks_field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_check_status_field: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validation_failed_values: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validation_passed_values: Vec<String>,
}

pub fn extract_task_universe_for_generated_run(
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
        task.dependency_ids = resolve_task_references(
            &task.dependency_ids,
            &aliases,
            &task.source_path,
            "dependency",
        )?;
        task.blocks_ids =
            resolve_task_references(&task.blocks_ids, &aliases, &task.source_path, "blocks")?;
        tasks.push(task);
    }
    tasks.sort_by(|left, right| left.canonical_task_id.cmp(&right.canonical_task_id));
    reconcile_blocks_into_dependencies(&mut tasks)?;
    validate_task_dependency_graph(&tasks)?;
    validate_declared_statuses(&tasks)?;

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

pub fn task_files_under(root: &Path) -> WorkflowResult<Vec<PathBuf>> {
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
        .filter(|part| is_absolute_path_token(part))
        .map(PathBuf::from)
        .collect()
}

/// Whether a whitespace-delimited token looks like an absolute path.
///
/// Previously this was `starts_with('/')`, which recognises POSIX paths only.
/// On Windows a task description naturally carries `C:\work\tasks\...`, none of
/// which matched, so no roots were ever extracted and every decomposed-PRD run
/// failed with "requires local TASK-*.md evidence before planning" — the
/// feature could not be used on that platform at all.
///
/// Accepting `C:\` and `C:/` on every platform is harmless: on Unix such a
/// token is not a real path, and the callers all probe the filesystem
/// (`is_dir`, `directory_has_task_files`) before believing it.
fn is_absolute_path_token(part: &str) -> bool {
    if part.starts_with('/') {
        return true;
    }
    let bytes = part.as_bytes();
    bytes.len() > 2
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
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
