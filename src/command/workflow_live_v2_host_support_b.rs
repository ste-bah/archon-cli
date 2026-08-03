use super::*;

pub(super) fn artifact_paths_exist(v2_root: &Path, paths: &[String]) -> bool {
    let concrete_paths = paths
        .iter()
        .map(|path| path.trim())
        .filter(|path| !path.is_empty())
        .filter(|path| !artifact_path_is_placeholder(path))
        .collect::<Vec<_>>();
    if concrete_paths.is_empty() {
        return paths.is_empty();
    }
    concrete_paths
        .iter()
        .all(|path| artifact_path_exists(v2_root, path))
}

pub(crate) fn artifact_path_exists(v2_root: &Path, path: &str) -> bool {
    if super::super::workflow_live_artifact_refs::is_nonfilesystem_artifact_ref(path) {
        return true;
    }
    let path = Path::new(path);
    if path.exists() {
        return true;
    }
    if path.is_absolute() {
        return false;
    }
    if v2_root
        .parent()
        .map(|run_root| run_root.join(path).exists())
        .unwrap_or(false)
    {
        return true;
    }
    project_root_for_v2(v2_root)
        .map(|project_root| project_root.join(path).exists())
        .unwrap_or(false)
        || repository_root_for_v2(v2_root)
            .map(|repo_root| repo_root.join(path).exists())
            .unwrap_or(false)
}

pub(super) fn project_root_for_v2(v2_root: &Path) -> Option<&Path> {
    let mut current = Some(v2_root);
    while let Some(path) = current {
        if path.file_name().and_then(|name| name.to_str()) == Some(".archon") {
            return path.parent();
        }
        current = path.parent();
    }
    None
}

pub(super) fn repository_root_for_v2(v2_root: &Path) -> Option<PathBuf> {
    let run_root = v2_root.parent()?;
    let state_path = run_root.join("state.json");
    let raw = fs::read_to_string(state_path).ok()?;
    let state: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let root = state
        .get("spec")
        .and_then(|spec| spec.get("target_repository_root"))
        .or_else(|| state.get("target_repository_root"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    Some(PathBuf::from(root))
}

pub(super) fn artifact_path_is_placeholder(path: &str) -> bool {
    path.contains('<') || path.contains('>') || path.contains('*')
}

pub(super) fn report_paths(v2_root: &Path) -> WorkflowV2ReportPaths {
    let run_root = v2_root.parent().unwrap_or(v2_root);
    WorkflowV2ReportPaths {
        harness_path: run_root.join("workflow.js").display().to_string(),
        run_state_path: run_root.join("state.json").display().to_string(),
        event_log_path: run_root.join("events.jsonl").display().to_string(),
    }
}

pub(super) fn artifact_paths_from_input(input: &serde_json::Value) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    collect_artifact_paths(input, &mut paths);
    paths
}

fn collect_artifact_paths(value: &serde_json::Value, paths: &mut Vec<PathBuf>) {
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                collect_artifact_paths(item, paths);
            }
        }
        serde_json::Value::Object(object) => {
            for key in ["path", "artifact_path", "artifactPath"] {
                if let Some(path) = object.get(key).and_then(serde_json::Value::as_str) {
                    paths.push(PathBuf::from(path));
                }
            }
            for key in [
                "artifacts",
                "artifact_paths",
                "artifactPaths",
                "required_artifacts",
                "requiredArtifacts",
                "source_data",
            ] {
                if let Some(items) = object.get(key) {
                    collect_artifact_paths(items, paths);
                }
            }
        }
        serde_json::Value::String(path) => paths.push(PathBuf::from(path)),
        _ => {}
    }
}

pub(super) fn artifact_path(v2_root: &Path, id: &str) -> PathBuf {
    v2_root
        .join("artifacts")
        .join(format!("{}.json", sanitize_id(id)))
}

pub(super) fn sanitize_id(raw: &str) -> String {
    raw.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

pub(super) fn write_json(path: &Path, value: &impl serde::Serialize) -> archon_workflow::WorkflowResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| WorkflowError::Io {
            path: parent.to_path_buf(),
            source: err,
        })?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(value)?).map_err(|err| WorkflowError::Io {
        path: tmp.clone(),
        source: err,
    })?;
    fs::rename(&tmp, path).map_err(|err| WorkflowError::Io {
        path: path.to_path_buf(),
        source: err,
    })
}
