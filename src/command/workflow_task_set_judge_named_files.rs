use super::*;
use std::path::Path;

#[cfg(test)]
pub(super) fn named_files(
    criterion: &archon_workflow::task_set_contract::AcceptanceCriterion,
    roots: &JudgeRoots,
) -> Vec<serde_json::Value> {
    let mut remaining = archon_workflow::check_source_settle::MAX_JUDGED_SOURCE_BYTES;
    named_files_with_budget(criterion, roots, &mut remaining)
}

/// Resolve the named source for one check while spending a shared prompt-batch
/// budget. Callers rendering a batch pass the same `remaining` value to each
/// check in prompt order.
pub(super) fn named_files_with_budget(
    criterion: &archon_workflow::task_set_contract::AcceptanceCriterion,
    roots: &JudgeRoots,
    remaining: &mut usize,
) -> Vec<serde_json::Value> {
    let root = match &criterion.check {
        archon_workflow::task_set_contract::AcceptanceCheck::Command {
            cwd: archon_workflow::task_set_contract::TrustedCwd::RepoRoot,
            ..
        } => roots.repository.as_deref(),
        _ => roots.project.as_deref(),
    };
    let Some(root) = root.and_then(|path| path.canonicalize().ok()) else {
        return Vec::new();
    };
    let project = roots
        .project
        .as_deref()
        .and_then(|path| path.canonicalize().ok())
        .unwrap_or_else(|| root.clone());
    let repository = roots
        .repository
        .as_deref()
        .and_then(|path| path.canonicalize().ok())
        .unwrap_or_else(|| root.clone());
    let task_root = roots
        .task_root
        .as_deref()
        .and_then(|path| path.canonicalize().ok());
    let project_input_excludes = task_root.as_deref().and_then(|task_root| {
        match crate::command::acceptance_scratch_policy::capture(&project, task_root) {
            Ok(Some(binding)) => Some(binding.policy.project_input_excludes),
            Ok(None) => Some(Vec::new()),
            Err(_) => None,
        }
    });
    let task_root_relative = task_root
        .as_deref()
        .and_then(|task_root| task_root.strip_prefix(&project).ok());
    let nested_repository =
        archon_workflow::acceptance_scratch::nested_repository(&project, &repository)
            .filter(|nested| !nested.as_os_str().is_empty());
    let mut inputs = crate::command::workflow_task_set::executability::named_inputs(
        criterion,
        &project,
        &repository,
    );
    inputs.sort();
    inputs.dedup();
    inputs
        .into_iter()
        .filter_map(|relative| {
            let Some(path) = normalize_named_path(Path::new(&relative)) else {
                return Some(serde_json::json!({
                    "path": archon_shell::paths::stored_text(Path::new(&relative)),
                    "status": "not_supplied"
                }));
            };
            let absolute = root.join(&path);
            let path_text = archon_shell::paths::stored_text(&path);
            if matches!(
                criterion.check,
                archon_workflow::task_set_contract::AcceptanceCheck::Command {
                    cwd: archon_workflow::task_set_contract::TrustedCwd::RepoRoot,
                    ..
                }
            ) {
                let Some(commit) = roots.repository_commit.as_deref() else {
                    return Some(repository_missing_entry(
                        &path_text,
                        &repository,
                        None,
                        &path_text,
                        &path,
                    ));
                };
                let stored_path = archon_shell::paths::stored_text(&path);
                let Some(content_bytes) =
                    repository_regular_blob(&repository, commit, &stored_path)
                else {
                    return Some(repository_missing_entry(
                        &path_text,
                        &repository,
                        Some(commit),
                        &stored_path,
                        &path,
                    ));
                };
                return Some(supplied(&path_text, &content_bytes, remaining));
            } else {
                match absolute.symlink_metadata() {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        return Some(serde_json::json!({"path": path_text, "status": "absent"}));
                    }
                    Err(_) => {
                        return Some(
                            serde_json::json!({"path": path_text, "status": "not_supplied"}),
                        );
                    }
                    Ok(_) => {}
                }
                let Ok(relative) = absolute.strip_prefix(&project) else {
                    return Some(serde_json::json!({"path": path_text, "status": "not_supplied"}));
                };
                if project_input_excludes.as_ref().is_none_or(|excludes| {
                    archon_workflow::acceptance_scratch::acceptance_input_excluded(
                        relative,
                        excludes,
                        task_root_relative,
                    )
                }) || nested_repository
                    .as_ref()
                    .is_some_and(|nested| relative.starts_with(nested))
                    || relative
                        .components()
                        .any(|component| component.as_os_str() == ".git")
                    || !project_file_is_copied(&project, relative)
                {
                    return Some(serde_json::json!({"path": path_text, "status": "not_supplied"}));
                }
            }
            let canonical = match absolute.canonicalize() {
                Ok(canonical) => canonical,
                Err(_) => {
                    return Some(serde_json::json!({"path": path_text, "status": "not_supplied"}));
                }
            };
            if !canonical.starts_with(&root) || !canonical.is_file() {
                return Some(serde_json::json!({"path": path_text, "status": "not_supplied"}));
            }
            // A file the host cannot read is reported, never dropped.
            let Ok(bytes) = std::fs::read(&canonical) else {
                return Some(serde_json::json!({"path": path_text, "status": "not_supplied"}));
            };
            Some(supplied(&path_text, &bytes, remaining))
        })
        .collect()
}

/// The entry for a named file whose `bytes` the host read: supplied whole
/// within the batch's `remaining` budget. A file the budget would cut is
/// not supplied and carries no partial content: a not_supplied input's
/// assertions are unknown to the judge, so none of them is shown, and it
/// spends none of the budget.
fn supplied(path: &str, bytes: &[u8], remaining: &mut usize) -> serde_json::Value {
    if bytes.len() > *remaining {
        return serde_json::json!({
            "path": path,
            "status": "not_supplied",
            "detail": "[not supplied: larger than the judged-source byte limit left for this batch]"
        });
    }
    *remaining -= bytes.len();
    let content =
        archon_observability::redaction::redact_secret_values(&String::from_utf8_lossy(bytes));
    serde_json::json!({"path": path, "status": "supplied", "content": content})
}

fn repository_missing_entry(
    path: &str,
    repository: &Path,
    commit: Option<&str>,
    stored_path: &str,
    relative: &Path,
) -> serde_json::Value {
    let working_path = repository.join(relative);
    if commit.is_some_and(|commit| repository_tree_entry(repository, commit, stored_path).is_some())
        || working_path.symlink_metadata().is_ok()
        || working_path.exists()
    {
        serde_json::json!({"path": path, "status": "not_supplied"})
    } else {
        serde_json::json!({"path": path, "status": "absent"})
    }
}

/// Digest the exact path/content list the judge prompt will receive.
pub(in crate::command::workflow_task_set) fn named_files_digest(
    criterion: &archon_workflow::task_set_contract::AcceptanceCriterion,
    roots: &JudgeRoots,
) -> String {
    let mut remaining = archon_workflow::check_source_settle::MAX_JUDGED_SOURCE_BYTES;
    named_files_digest_with_budget(criterion, roots, &mut remaining)
}

pub(in crate::command::workflow_task_set) fn named_files_digest_with_budget(
    criterion: &archon_workflow::task_set_contract::AcceptanceCriterion,
    roots: &JudgeRoots,
    remaining: &mut usize,
) -> String {
    let mut resolved = roots.clone();
    if resolved.repository_commit.is_none() {
        if let Some(repository) = resolved
            .repository
            .as_deref()
            .and_then(|p| p.canonicalize().ok())
        {
            let output = archon_shell::spawn::command("git")
                .arg("-C")
                .arg(repository)
                .args(["rev-parse", "HEAD"])
                .output();
            resolved.repository_commit = output.ok().and_then(|output| {
                output
                    .status
                    .success()
                    .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
            });
        }
    }
    named_files_digest_for_list(&named_files_with_budget(criterion, &resolved, remaining))
}

pub(in crate::command::workflow_task_set) fn named_files_digest_for_list(
    files: &[serde_json::Value],
) -> String {
    content_digest(
        &serde_json::to_vec(files).expect("named judge files are serializable JSON values"),
    )
}

fn normalize_named_path(path: &Path) -> Option<PathBuf> {
    use std::path::Component;
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(name) => normalized.push(name),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    (!normalized.as_os_str().is_empty()).then_some(normalized)
}

fn repository_regular_blob(repository: &Path, commit: &str, path: &str) -> Option<Vec<u8>> {
    let entry = repository_tree_entry(repository, commit, path)?;
    // Git regular files are blobs with mode 100644 or 100755. In particular,
    // mode 120000 is a symlink blob and must never be followed or shown.
    if !(entry.starts_with(b"100644 blob ") || entry.starts_with(b"100755 blob ")) {
        return None;
    }
    let output = archon_shell::spawn::command("git")
        .arg("-C")
        .arg(repository)
        .args(["cat-file", "blob", &format!("{commit}:{path}")])
        .output()
        .ok()?;
    output.status.success().then_some(output.stdout)
}

fn repository_tree_entry(repository: &Path, commit: &str, path: &str) -> Option<Vec<u8>> {
    let tree_entry = archon_shell::spawn::command("git")
        .arg("-C")
        .arg(repository)
        .args(["ls-tree", "-z", commit, "--", &format!(":(literal){path}")])
        .output()
        .ok()?;
    if !tree_entry.status.success() {
        return None;
    }
    let entry = tree_entry
        .stdout
        .split(|byte| *byte == 0)
        .find_map(|record| {
            let separator = record.iter().position(|byte| *byte == b'\t')?;
            let (metadata, entry_path_with_separator) = record.split_at(separator);
            let entry_path = entry_path_with_separator.get(1..)?;
            (entry_path == path.as_bytes()).then_some(metadata.to_vec())
        })?;
    Some(entry)
}

fn project_file_is_copied(project_root: &Path, relative: &Path) -> bool {
    use std::path::Component;
    if relative
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return false;
    }
    let Ok(root) = project_root.canonicalize() else {
        return false;
    };
    let candidate = root.join(relative);
    if candidate.canonicalize().ok().as_deref() != Some(candidate.as_path()) {
        return false;
    }
    let mut current = project_root.to_path_buf();
    let mut components = relative.components().peekable();
    while let Some(std::path::Component::Normal(name)) = components.next() {
        current.push(name);
        let Ok(metadata) = current.symlink_metadata() else {
            return false;
        };
        if metadata.file_type().is_symlink() {
            return false;
        }
        if components.peek().is_some() {
            if !metadata.is_dir() {
                return false;
            }
        } else if !metadata.is_file() {
            return false;
        }
    }
    !relative.as_os_str().is_empty()
}
