use std::path::{Component, Path, PathBuf};

use super::{
    WorkflowV2Artifact, WorkflowV2ProjectArtifactContext, WorkflowV2Result,
    WorkflowV2WriteSafetyError, branch_evidence::semantic_branch_result_from_value,
};

const MAX_ARTIFACT_RESULT_FILES: usize = 256;
const MAX_ARTIFACT_RESULT_BYTES: u64 = 1_048_576;

struct BranchResultCandidate {
    path: String,
    score: usize,
    result: WorkflowV2Result,
}

pub fn load_project_artifact_branch_result(
    call_id: &str,
    context: &WorkflowV2ProjectArtifactContext,
) -> Result<Option<WorkflowV2Result>, WorkflowV2WriteSafetyError> {
    let Some(project_root) = project_root(context) else {
        return Ok(None);
    };
    let roots = artifact_result_roots(&project_root, context)?;
    let mut candidates = Vec::new();
    for root in roots {
        scan_artifact_root(call_id, &project_root, &root, &mut candidates)?;
    }
    candidates.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.path.cmp(&right.path))
    });
    Ok(candidates
        .into_iter()
        .next()
        .map(|candidate| candidate.result))
}

fn artifact_result_roots(
    project_root: &Path,
    context: &WorkflowV2ProjectArtifactContext,
) -> Result<Vec<PathBuf>, WorkflowV2WriteSafetyError> {
    let mut roots = Vec::new();
    for raw in &context.artifact_roots {
        let relative = normalize_relative(raw)?;
        if artifact_result_root_allowed(&relative) {
            roots.push(project_root.join(relative));
        }
    }
    if let Some(run_id) = context.run_id.as_deref().filter(|id| !id.is_empty()) {
        roots.push(
            project_root
                .join(".archon")
                .join("workflows")
                .join(run_id)
                .join("artifacts"),
        );
    }
    roots.sort();
    roots.dedup();
    Ok(roots)
}

fn artifact_result_root_allowed(relative: &str) -> bool {
    relative == "artifacts"
        || relative == ".archon/artifacts"
        || relative.ends_with("/artifacts")
        || relative.contains("/artifacts/")
}

fn scan_artifact_root(
    call_id: &str,
    project_root: &Path,
    root: &Path,
    candidates: &mut Vec<BranchResultCandidate>,
) -> Result<(), WorkflowV2WriteSafetyError> {
    if !root.exists() {
        return Ok(());
    }
    ensure_under_project(project_root, root)?;
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        for entry in std::fs::read_dir(&path).map_err(|_| unsafe_target(&path))? {
            let entry = entry.map_err(|_| unsafe_target(&path))?;
            let file_type = entry.file_type().map_err(|_| unsafe_target(&path))?;
            if file_type.is_dir() {
                stack.push(entry.path());
            } else if artifact_json_file(&entry.path()) {
                maybe_push_candidate(call_id, project_root, &entry.path(), candidates)?;
            }
            if candidates.len() >= MAX_ARTIFACT_RESULT_FILES {
                return Ok(());
            }
        }
    }
    Ok(())
}

fn maybe_push_candidate(
    call_id: &str,
    project_root: &Path,
    path: &Path,
    candidates: &mut Vec<BranchResultCandidate>,
) -> Result<(), WorkflowV2WriteSafetyError> {
    ensure_under_project(project_root, path)?;
    let metadata = std::fs::metadata(path).map_err(|_| unsafe_target(path))?;
    if metadata.len() > MAX_ARTIFACT_RESULT_BYTES {
        return Ok(());
    }
    let Ok(body) = std::fs::read_to_string(path) else {
        return Ok(());
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&body) else {
        return Ok(());
    };
    if let Some(mut result) = branch_result_from_value(value) {
        let display = display_path(project_root, path);
        attach_result_artifact(&mut result, &display);
        candidates.push(BranchResultCandidate {
            score: artifact_result_score(call_id, &display),
            path: display,
            result,
        });
    }
    Ok(())
}

fn branch_result_from_value(value: serde_json::Value) -> Option<WorkflowV2Result> {
    if branch_result_marker(&value) {
        return parse_branch_result(value);
    }
    let object = value.as_object()?;
    if wrapper_branch_result_marker(&value) {
        return object
            .get("branch_result")
            .cloned()
            .and_then(parse_branch_result);
    }
    semantic_branch_result_from_value(&value)
}

fn parse_branch_result(value: serde_json::Value) -> Option<WorkflowV2Result> {
    let mut result = serde_json::from_value::<WorkflowV2Result>(value.clone()).ok()?;
    copy_top_level_branch_fields(&mut result, &value);
    Some(result)
}

fn branch_result_marker(value: &serde_json::Value) -> bool {
    value
        .get("schema")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|schema| schema.contains("workflow_v2_branch_result"))
}

fn wrapper_branch_result_marker(value: &serde_json::Value) -> bool {
    value
        .get("schema")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|schema| schema.contains("branch_result"))
}

fn copy_top_level_branch_fields(result: &mut WorkflowV2Result, value: &serde_json::Value) {
    let mut data = result.data.as_object().cloned().unwrap_or_default();
    for key in ["item_id", "source_item_id", "canonical_task_ids"] {
        if data.get(key).is_none() {
            if let Some(raw) = value.get(key) {
                data.insert(key.to_string(), raw.clone());
            }
        }
    }
    result.data = serde_json::Value::Object(data);
}

fn attach_result_artifact(result: &mut WorkflowV2Result, path: &str) {
    if result
        .artifacts
        .iter()
        .any(|artifact| artifact.path == path)
    {
        return;
    }
    result.artifacts.push(WorkflowV2Artifact {
        id: artifact_id(path),
        path: path.to_string(),
        description: Some("typed workflow branch result artifact".to_string()),
    });
}

fn artifact_result_score(call_id: &str, path: &str) -> usize {
    path.split('/')
        .filter(|part| !part.is_empty() && call_id.starts_with(*part))
        .map(str::len)
        .max()
        .unwrap_or(0)
}

fn artifact_json_file(path: &Path) -> bool {
    path.extension().and_then(|ext| ext.to_str()) == Some("json")
}

fn display_path(project_root: &Path, path: &Path) -> String {
    path.strip_prefix(project_root)
        .map(|relative| relative.to_string_lossy().to_string())
        .unwrap_or_else(|_| path.display().to_string())
}

fn project_root(context: &WorkflowV2ProjectArtifactContext) -> Option<PathBuf> {
    context
        .project_root
        .as_deref()
        .filter(|root| !root.trim().is_empty())
        .map(PathBuf::from)
}

fn normalize_relative(raw: &str) -> Result<String, WorkflowV2WriteSafetyError> {
    let mut parts = Vec::new();
    for component in Path::new(raw.trim()).components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().to_string()),
            Component::CurDir => {}
            _ => return Err(unsafe_target(Path::new(raw))),
        }
    }
    (!parts.is_empty())
        .then(|| parts.join("/"))
        .ok_or_else(|| unsafe_target(Path::new(raw)))
}

fn ensure_under_project(
    project_root: &Path,
    path: &Path,
) -> Result<(), WorkflowV2WriteSafetyError> {
    let root = std::fs::canonicalize(project_root).map_err(|_| unsafe_target(project_root))?;
    let candidate = std::fs::canonicalize(path).map_err(|_| unsafe_target(path))?;
    if candidate.starts_with(root) {
        Ok(())
    } else {
        Err(unsafe_target(path))
    }
}

fn artifact_id(path: &str) -> String {
    let id = path
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect::<String>();
    id.trim_matches('-').to_string()
}

fn unsafe_target(path: &Path) -> WorkflowV2WriteSafetyError {
    WorkflowV2WriteSafetyError::UnsafeTarget {
        item_id: "project-artifact-branch-result".to_string(),
        target: path.display().to_string(),
    }
}
