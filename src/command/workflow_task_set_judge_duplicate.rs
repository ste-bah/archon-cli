use super::*;

fn normalized(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                result.pop();
            }
            std::path::Component::CurDir => {}
            other => result.push(other.as_os_str()),
        }
    }
    result
}

fn check_root(criterion: &AcceptanceCriterion, roots: &JudgeRoots) -> Option<PathBuf> {
    let root = match criterion.check {
        archon_workflow::task_set_contract::AcceptanceCheck::Command {
            cwd: archon_workflow::task_set_contract::TrustedCwd::RepoRoot,
            ..
        } => roots.repository.as_deref(),
        _ => roots.project.as_deref(),
    }?;
    Some(root.canonicalize().unwrap_or_else(|_| root.to_path_buf()))
}

pub(in crate::command::workflow_task_set::judge) fn same_path(
    left: &str,
    right: &str,
    criterion: &AcceptanceCriterion,
    roots: &JudgeRoots,
) -> bool {
    check_root(criterion, roots)
        .is_some_and(|root| normalized(&root.join(left)) == normalized(&root.join(right)))
}

fn repository_path(
    path: &str,
    criterion: &AcceptanceCriterion,
    roots: &JudgeRoots,
) -> Option<String> {
    // Normalize lexically: a committed base-tree file may be absent from the
    // live working tree. Never follow a live symlink to decide base existence.
    let absolute = normalized(&check_root(criterion, roots)?.join(path));
    let repository = roots.repository.as_deref()?;
    let repository = repository
        .canonicalize()
        .unwrap_or_else(|_| repository.to_path_buf());
    Some(archon_shell::paths::stored_text(
        absolute.strip_prefix(repository).ok()?,
    ))
}

/// A duplicate must name an existing path in the committed base tree.
pub(in crate::command::workflow_task_set::judge) fn exists_in_base(
    path: &str,
    criterion: &AcceptanceCriterion,
    roots: &JudgeRoots,
) -> bool {
    repository_path(path, criterion, roots).is_some_and(|relative| {
        roots
            .repository
            .as_deref()
            .zip(roots.repository_commit.as_deref())
            .is_some_and(|(repository, commit)| {
                repository_tree_entry(repository, commit, &relative).is_some()
            })
    })
}

/// Only structured existing checks can prove equivalence. Source text or a
/// command appearing as a substring of another command cannot prove it.
pub(in crate::command::workflow_task_set::judge) fn same_check_in_base(
    path: &str,
    criterion: &AcceptanceCriterion,
    roots: &JudgeRoots,
) -> bool {
    let Some(relative) = repository_path(path, criterion, roots) else {
        return false;
    };
    let Some((repository, commit)) = roots
        .repository
        .as_deref()
        .zip(roots.repository_commit.as_deref())
    else {
        return false;
    };
    let Some(bytes) = repository_regular_blob(repository, commit, &relative) else {
        return false;
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return false;
    };
    matching_check(&value, criterion, roots)
}

fn matching_check(
    value: &serde_json::Value,
    criterion: &AcceptanceCriterion,
    roots: &JudgeRoots,
) -> bool {
    use archon_workflow::task_set_contract::AcceptanceCheck;
    if let Some(check) = value.get("check")
        && let Ok(check) = serde_json::from_value::<AcceptanceCheck>(check.clone())
        && let (
            AcceptanceCheck::Command { command: left, .. },
            AcceptanceCheck::Command { command: right, .. },
        ) = (&check, &criterion.check)
    {
        let mut existing = criterion.clone();
        existing.check = check.clone();
        if left.replace("\r\n", "\n") == right.replace("\r\n", "\n")
            && check_root(&existing, roots).is_some()
            && check_root(&existing, roots) == check_root(criterion, roots)
        {
            return true;
        }
    }
    ["acceptance", "supplementary"].iter().any(|key| {
        value
            .get(key)
            .and_then(serde_json::Value::as_array)
            .is_some_and(|entries| {
                entries
                    .iter()
                    .any(|entry| matching_check(entry, criterion, roots))
            })
    })
}
