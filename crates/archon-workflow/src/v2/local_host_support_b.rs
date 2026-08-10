use super::*;

/// Does every declared artifact path of a completion-evidence item resolve to
/// real evidence?
///
/// # Issue #168: a directory is not the artifact
///
/// This delegated to [`artifact_path_exists`], which answers a plain "is there
/// something at this path" — true for a directory and true for a zero-byte
/// file. Task-completion credit is granted on the strength of this answer, so a
/// criterion-named directory left in the project root could buy credit for the
/// task whose criterion named it. Credit now requires a regular, non-empty
/// file; the plain-existence predicate is kept for
/// [`super::local_host_support_a::contradicted_existence_claims`], where "does
/// this path exist" really is the question being asked.
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
        .all(|path| artifact_path_is_evidence(v2_root, path))
}

/// [`artifact_path_exists`], narrowed to a regular non-empty file.
pub(super) fn artifact_path_is_evidence(v2_root: &Path, path: &str) -> bool {
    if crate::v2::artifact_refs::is_nonfilesystem_artifact_ref(path) {
        return true;
    }
    resolved_artifact_candidates(v2_root, Path::new(path))
        .iter()
        .any(|candidate| crate::v2::artifact_path_guard::artifact_file_is_evidence(candidate))
}

/// Every location a declared artifact path could name, in resolution order.
fn resolved_artifact_candidates(v2_root: &Path, path: &Path) -> Vec<PathBuf> {
    let mut candidates = vec![path.to_path_buf()];
    if path.is_absolute() {
        return candidates;
    }
    candidates.extend(v2_root.parent().map(|run_root| run_root.join(path)));
    candidates.extend(project_root_for_v2(v2_root).map(|root| root.join(path)));
    candidates.extend(repository_root_for_v2(v2_root).map(|root| root.join(path)));
    candidates
}

pub(super) fn artifact_path_exists(v2_root: &Path, path: &str) -> bool {
    if crate::v2::artifact_refs::is_nonfilesystem_artifact_ref(path) {
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

/// A path that names a family rather than a file, and so cannot be checked
/// literally.
///
/// Deliberately NOT extended to `${...}` for issue #168. A shell-templated path
/// stays concrete here, fails the literal check, and denies credit. Classifying
/// it as a placeholder would make it *skipped* instead — vacuously satisfied
/// alongside any one real path in the same list, which is the direction the
/// fix is trying to close, not open.
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

pub(super) fn write_json(path: &Path, value: &impl serde::Serialize) -> crate::WorkflowResult<()> {
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
