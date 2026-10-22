use super::*;
use std::path::Path;

pub(super) fn named_files(
    criterion: &archon_workflow::task_set_contract::AcceptanceCriterion,
    roots: &JudgeRoots,
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
    let mut remaining = archon_workflow::check_source_settle::MAX_JUDGED_SOURCE_BYTES;
    inputs
        .into_iter()
        .filter_map(|relative| {
            let path = normalize_named_path(Path::new(&relative))?;
            let absolute = root.join(&path);
            if matches!(
                criterion.check,
                archon_workflow::task_set_contract::AcceptanceCheck::Command {
                    cwd: archon_workflow::task_set_contract::TrustedCwd::RepoRoot,
                    ..
                }
            ) {
                let commit = roots.repository_commit.as_deref()?;
                let stored_path = archon_shell::paths::stored_text(&path);
                let Some(content_bytes) =
                    repository_regular_blob(&repository, commit, &stored_path)
                else {
                    return None;
                };
                let bytes = content_bytes;
                let take = bytes.len().min(remaining);
                let take = utf8_prefix_len(&bytes, take);
                let truncated = take < bytes.len();
                let shown = &bytes[..take];
                remaining -= take;
                let mut content = archon_observability::redaction::redact_secret_values(
                    &String::from_utf8_lossy(shown),
                );
                if truncated {
                    content.push_str("\n[truncated at the existing judged-source byte limit]");
                }
                return Some(
                    serde_json::json!({"path": archon_shell::paths::stored_text(path), "content": content}),
                );
            } else {
                let canonical = absolute.canonicalize().ok()?;
                if !canonical.starts_with(&root) || !canonical.is_file() {
                    return None;
                }
                let relative = absolute.strip_prefix(&project).ok()?;
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
                    return None;
                }
            }
            let canonical = absolute.canonicalize().ok()?;
            let bytes = std::fs::read(&canonical).ok()?;
            let take = bytes.len().min(remaining);
            let take = utf8_prefix_len(&bytes, take);
            let truncated = take < bytes.len();
            let shown = &bytes[..take];
            remaining -= take;
            let mut content = archon_observability::redaction::redact_secret_values(
                &String::from_utf8_lossy(shown),
            );
            if truncated {
                content.push_str("\n[truncated at the existing judged-source byte limit]");
            }
            Some(serde_json::json!({
                "path": archon_shell::paths::stored_text(path),
                "content": content,
            }))
        })
        .collect()
}

/// Digest the exact path/content list the judge prompt will receive.
pub(in crate::command::workflow_task_set) fn named_files_digest(
    criterion: &archon_workflow::task_set_contract::AcceptanceCriterion,
    roots: &JudgeRoots,
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
    content_digest(
        &serde_json::to_vec(&named_files(criterion, &resolved))
            .expect("named judge files are serializable JSON values"),
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

fn utf8_prefix_len(bytes: &[u8], limit: usize) -> usize {
    let limit = limit.min(bytes.len());
    match std::str::from_utf8(&bytes[..limit]) {
        Ok(_) => limit,
        Err(error) if error.error_len().is_none() => error.valid_up_to(),
        Err(_) => limit,
    }
}

fn repository_regular_blob(repository: &Path, commit: &str, path: &str) -> Option<Vec<u8>> {
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
            (entry_path == path.as_bytes()).then_some(metadata)
        })?;
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
