use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::run::WorkflowRun;
use crate::source_context;
use crate::spec::StageSpec;
use crate::store::WorkflowStore;

pub(crate) const REQUIRED_ARTIFACT_INVENTORY_TOOL: &str = "required_artifact_inventory";

const ARTIFACT_KEYS: &[&str] = &[
    "required_artifacts",
    "required_artifact_paths",
    "required_output_files",
    "required_deliverables",
    "deliverable_files",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RequiredArtifactReport {
    pub required: Vec<String>,
    pub checked: Vec<CheckedArtifact>,
    pub missing: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CheckedArtifact {
    pub requested: String,
    pub resolved: String,
    pub exists: bool,
}

impl RequiredArtifactReport {
    pub(crate) fn failure_reason(&self) -> Option<String> {
        if self.missing.is_empty() {
            return None;
        }
        Some(format!(
            "quality gate missing required artifact(s): {}",
            self.missing.join(", ")
        ))
    }
}

pub(crate) fn check_required_artifacts(
    store: &WorkflowStore,
    run: &WorkflowRun,
    stage: &StageSpec,
) -> Option<RequiredArtifactReport> {
    let required = required_artifact_paths(stage);
    if required.is_empty() {
        return None;
    }
    let roots = artifact_roots(store, run);
    let checked = required
        .iter()
        .map(|path| check_artifact_path(&roots, path))
        .collect::<Vec<_>>();
    let missing = checked
        .iter()
        .filter(|artifact| !artifact.exists)
        .map(|artifact| artifact.requested.clone())
        .collect();
    Some(RequiredArtifactReport {
        required,
        checked,
        missing,
    })
}

pub(crate) fn required_artifact_paths(stage: &StageSpec) -> Vec<String> {
    let mut paths = Vec::new();
    for key in ARTIFACT_KEYS {
        collect_artifact_paths(stage.extra.get(*key), &mut paths);
        collect_artifact_paths(stage.input.get(*key), &mut paths);
    }
    paths.sort();
    paths.dedup();
    paths
}

pub(crate) fn inventory_body(
    store: &WorkflowStore,
    run: &WorkflowRun,
    stage: &StageSpec,
) -> String {
    let report = check_required_artifacts(store, run, stage).unwrap_or_else(empty_report);
    let project = project_root(store);
    let items = report
        .checked
        .iter()
        .filter(|artifact| !artifact.exists)
        .map(|artifact| inventory_item(&project, run, artifact))
        .collect::<Vec<_>>();
    serde_json::json!({
        "items": items,
        "required_artifacts": report.required,
        "missing": report.missing,
    })
    .to_string()
}

fn collect_artifact_paths(value: Option<&Value>, paths: &mut Vec<String>) {
    match value {
        Some(Value::String(text)) if looks_like_artifact_path(text) => {
            paths.push(text.trim().to_string());
        }
        Some(Value::Array(items)) => {
            items
                .iter()
                .for_each(|item| collect_artifact_paths(Some(item), paths));
        }
        Some(Value::Object(map)) => {
            for key in ["path", "file", "artifact", "output", "target"] {
                collect_artifact_paths(map.get(key), paths);
            }
        }
        _ => {}
    }
}

fn looks_like_artifact_path(text: &str) -> bool {
    let text = text.trim();
    !text.is_empty()
        && (text.starts_with('/')
            || text.starts_with('.')
            || text.contains('/')
            || artifact_extension(text))
}

fn artifact_extension(text: &str) -> bool {
    [
        ".csv", ".db", ".duckdb", ".json", ".md", ".parquet", ".pine", ".sqlite", ".toml", ".txt",
        ".yaml", ".yml",
    ]
    .iter()
    .any(|ext| text.ends_with(ext))
}

fn empty_report() -> RequiredArtifactReport {
    RequiredArtifactReport {
        required: Vec::new(),
        checked: Vec::new(),
        missing: Vec::new(),
    }
}

fn inventory_item(
    project: &Path,
    run: &WorkflowRun,
    artifact: &CheckedArtifact,
) -> serde_json::Value {
    let root = target_root(project, run, &artifact.resolved);
    serde_json::json!({
        "kind": "missing_required_artifact",
        "artifact_path": artifact.requested,
        "resolved_path": artifact.resolved,
        "target_files": [artifact.resolved],
        "target_repository_root": root.display().to_string(),
        "task": format!(
            "Create or repair the required workflow artifact `{}` at `{}` using upstream PRD, task, test, and review evidence. Do not create placeholder content.",
            artifact.requested,
            artifact.resolved
        ),
        "required_tests": [],
    })
}

fn target_root(project: &Path, run: &WorkflowRun, resolved: &str) -> PathBuf {
    let path = Path::new(resolved);
    if path.starts_with(project) {
        return project.to_path_buf();
    }
    if let Some(root) = configured_repository_root(run, project)
        && path.starts_with(&root)
    {
        return root;
    }
    path.parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| project.to_path_buf())
}

fn artifact_roots(store: &WorkflowStore, run: &WorkflowRun) -> Vec<PathBuf> {
    let project = project_root(store);
    let mut roots = vec![project.clone()];
    if let Some(root) = configured_repository_root(run, &project) {
        roots.push(root);
    }
    roots.push(source_context::effective_root(store, run));
    dedupe_paths(&mut roots);
    roots
}

fn configured_repository_root(run: &WorkflowRun, project: &Path) -> Option<PathBuf> {
    let root = run
        .spec
        .target_repository_root
        .as_deref()
        .map(str::trim)
        .filter(|root| !root.is_empty())?;
    let path = Path::new(root);
    Some(if path.is_absolute() {
        path.to_path_buf()
    } else {
        project.join(path)
    })
}

fn check_artifact_path(roots: &[PathBuf], requested: &str) -> CheckedArtifact {
    let candidates = path_candidates(roots, requested);
    let resolved = candidates
        .iter()
        .find(|path| path.exists())
        .or_else(|| candidates.first())
        .cloned()
        .unwrap_or_else(|| PathBuf::from(requested));
    CheckedArtifact {
        requested: requested.to_string(),
        resolved: resolved.display().to_string(),
        exists: resolved.exists(),
    }
}

fn path_candidates(roots: &[PathBuf], requested: &str) -> Vec<PathBuf> {
    let path = Path::new(requested.trim());
    if path.is_absolute() {
        return vec![path.to_path_buf()];
    }
    let roots = ordered_roots_for(requested, roots);
    roots.into_iter().map(|root| root.join(path)).collect()
}

fn ordered_roots_for(requested: &str, roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut ordered = roots.to_vec();
    if requested.trim().starts_with(".archon/") {
        ordered.sort_by_key(|root| !root.join(".archon").is_dir());
    } else if ordered.len() > 1 {
        ordered.rotate_left(1);
    }
    ordered
}

fn project_root(store: &WorkflowStore) -> PathBuf {
    let root = store.root();
    if root.file_name().is_some_and(|name| name == "workflows")
        && root
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|name| name == ".archon")
    {
        return root
            .parent()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .unwrap_or_else(|| root.to_path_buf());
    }
    root.parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| root.to_path_buf())
}

fn dedupe_paths(paths: &mut Vec<PathBuf>) {
    let mut seen = std::collections::BTreeSet::new();
    paths.retain(|path| seen.insert(path.display().to_string()));
}
