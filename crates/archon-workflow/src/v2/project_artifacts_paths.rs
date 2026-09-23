//! Path safety for project artifacts: confining every resolved path to the
//! project root, and rejecting the ones that escape it.
//!
//! A child module of `project_artifacts` rather than a sibling, so these stay
//! reachable only from there. Split out because each source file in this tree
//! is held under a 500-line ceiling.

use super::*;

pub(super) fn ensure_existing_project_path(
    item_id: &str,
    project_root: &Path,
    absolute: &Path,
    relative: &str,
) -> Result<(), WorkflowV2WriteSafetyError> {
    let Ok(canonical_project) = std::fs::canonicalize(project_root) else {
        return Err(unsafe_target(item_id, relative));
    };
    let canonical_path =
        std::fs::canonicalize(absolute).map_err(|_| unsafe_target(item_id, relative))?;
    if canonical_path.starts_with(canonical_project) {
        Ok(())
    } else {
        Err(unsafe_target(item_id, relative))
    }
}

pub(super) fn ensure_project_path_parent_safe(
    item_id: &str,
    project_root: &Path,
    absolute: &Path,
    relative: &str,
) -> Result<(), WorkflowV2WriteSafetyError> {
    let parent =
        nearest_existing_parent(absolute).ok_or_else(|| unsafe_target(item_id, relative))?;
    ensure_existing_project_path(item_id, project_root, &parent, relative)
}

pub(super) fn nearest_existing_parent(path: &Path) -> Option<PathBuf> {
    let mut current = path.parent();
    while let Some(parent) = current {
        if parent.exists() {
            return Some(parent.to_path_buf());
        }
        current = parent.parent();
    }
    None
}

pub(super) fn absolute_artifact_candidate(
    project_root: &Path,
    relative: &str,
    context: &WorkflowV2ProjectArtifactContext,
) -> PathBuf {
    if let Some(run_id) = context.run_id.as_deref().filter(|id| !id.is_empty())
        && relative.starts_with("artifacts/")
    {
        return project_root
            .join(".archon")
            .join("workflows")
            .join(run_id)
            .join(relative);
    }
    project_root.join(relative)
}

/// Did the branch produce this declared artifact in its OWN working tree?
///
/// EXACTLY TWO locations are consulted for a declared project artifact, and
/// nothing else is looked at:
///
/// 1. the destination project root (`context.project_root`), resolved by
///    [`absolute_artifact_candidate`] for `project_artifact_status` — the
///    tree the run's deliverables finally live in;
/// 2. the working tree the call was dispatched into
///    (`context.repository_root`), resolved as a plain join — for a worktree
///    write branch, its isolated worktree.
///
/// Both are places the branch itself writes, which is why consulting the
/// second cannot launder a false report: a claim is still refused unless a
/// real, non-empty file — or a non-empty directory, where the contract
/// declares one — is on disk in a tree the branch could have put it in. An
/// artifact produced in neither is still reported absent, with the same
/// defect wording it had before.
///
/// # The ordering this exists for
///
/// A write branch runs in an isolated working tree, and a project deliverable
/// it generates there reaches the destination project root only later, when
/// the branch's output is applied. Validating the branch's REPORT against the
/// destination alone therefore asks whether a file is somewhere it cannot be
/// yet, and answers no for work that was genuinely done.
///
/// Observed live: a branch generated a timestamped project-data snapshot
/// inside its worktree, declared it, and was failed for "it does not exist" —
/// twice on the same task, on two different timestamped files, each costing
/// that task one of its six repair attempts. The destination copy of the file
/// the run named was created twenty-one minutes after the branch generated
/// it. The correct code in that branch — a new test plus a support module the
/// host's own under-reporting gap had already named — was discarded along
/// with the report it was failed for.
///
/// The sibling check in `project_artifact_completion` has consulted this same
/// second root for the declared-artifact CONTRACT since the source-deliverable
/// fix; this is the reporting path catching up, so the two no longer disagree
/// about whether one file exists.
pub(super) fn branch_produced_artifact(
    item_id: &str,
    project_root: &Path,
    relative: &str,
    context: &WorkflowV2ProjectArtifactContext,
) -> bool {
    let Some(branch_root) = branch_working_root(project_root, context) else {
        return false;
    };
    // `relative` has already passed `normalize_relative_path`: no `..`, no
    // root, no prefix, so the join cannot leave `branch_root` by spelling.
    // `ensure_existing_project_path` canonicalizes and re-checks containment
    // so it cannot leave by symlink either — a link out of the branch tree is
    // not the branch's own output.
    let candidate = branch_root.join(relative);
    declared_artifact_defect(
        relative,
        &candidate,
        context.declared_as_directory(relative),
    )
    .is_none()
        && ensure_existing_project_path(item_id, &branch_root, &candidate, relative).is_ok()
}

/// The working tree the BRANCH was dispatched into, when that is a different
/// tree from the destination project root.
///
/// `repository_root` is the directory the call actually ran in: for a worktree
/// write branch its isolated worktree, for every other call the run's target
/// repository. Offered for existence checks only — it grants no write
/// anywhere, and write confinement still answers to the project root alone.
/// `project_artifact_completion::repository_candidate` is the same idea for
/// the declared-artifact contract; this is the same second location, reached
/// from the path-classification side.
fn branch_working_root(
    project_root: &Path,
    context: &WorkflowV2ProjectArtifactContext,
) -> Option<PathBuf> {
    let root = strip_verbatim_prefix(context.repository_root.as_deref()?.trim());
    if root.is_empty() {
        return None;
    }
    let root = PathBuf::from(root);
    (root != project_root).then_some(root)
}

pub(super) fn clean_absolute_artifact_path(
    item_id: &str,
    raw: &str,
) -> Result<PathBuf, WorkflowV2WriteSafetyError> {
    let path = Path::new(raw);
    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::RootDir | Component::Prefix(_) | Component::Normal(_) => {
                clean.push(component.as_os_str())
            }
            Component::CurDir => {}
            Component::ParentDir => return Err(unsafe_target(item_id, raw)),
        }
    }
    Ok(clean)
}

pub(super) fn normalize_relative_path(
    item_id: &str,
    raw: &str,
) -> Result<String, WorkflowV2WriteSafetyError> {
    let path = Path::new(raw.trim());
    if raw.trim().is_empty() || path.is_absolute() {
        return Err(unsafe_target(item_id, raw));
    }
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().to_string()),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(unsafe_target(item_id, raw));
            }
        }
    }
    (!parts.is_empty())
        .then(|| parts.join("/"))
        .ok_or_else(|| unsafe_target(item_id, raw))
}

/// Windows `canonicalize()` returns a verbatim path (`\\?\C:\...`), and the
/// `?` in that prefix is indistinguishable from a glob to
/// `artifact_path_is_templated`, so every canonicalized absolute path was
/// classified as an unexpanded template: dropped from `files_changed`,
/// never recorded as an artifact, and raising a blocking
/// `unexpanded_artifact_template_*` gap. The prefix is a host addressing
/// detail, not part of the path the contract names. Stripped from both the
/// reported path and the project root, or the two no longer share a prefix.
pub(super) fn strip_verbatim_prefix(path: &str) -> &str {
    path.strip_prefix(r"\\?\UNC\")
        .or_else(|| path.strip_prefix(r"\\?\"))
        .unwrap_or(path)
}

#[cfg(test)]
mod verbatim_prefix_tests {
    use super::strip_verbatim_prefix;

    /// Asserted on the prefix directly, not through classification. A
    /// mistyped escape here still reads as a plausible fix and still strips
    /// nothing; the only thing that caught it was an unrelated end-to-end
    /// assertion four layers away.
    #[test]
    fn a_verbatim_drive_prefix_is_not_part_of_the_path() {
        assert_eq!(
            strip_verbatim_prefix(r"\\?\C:\proj\docs\audit.md"),
            r"C:\proj\docs\audit.md"
        );
    }

    #[test]
    fn a_verbatim_unc_prefix_leaves_the_share_path() {
        assert_eq!(
            strip_verbatim_prefix(r"\\?\UNC\server\share\audit.md"),
            r"server\share\audit.md"
        );
    }

    #[test]
    fn an_ordinary_path_is_returned_untouched() {
        assert_eq!(strip_verbatim_prefix("docs/audit.md"), "docs/audit.md");
        assert_eq!(
            strip_verbatim_prefix(r"C:\proj\docs\audit.md"),
            r"C:\proj\docs\audit.md"
        );
    }

    /// The `?` this strips is the verbatim marker alone. A genuine glob still
    /// has to reach `artifact_path_is_templated` intact, or stripping the
    /// prefix would smuggle unexpanded templates past the check it exists to
    /// keep honest.
    #[test]
    fn a_real_glob_survives_stripping() {
        assert_eq!(
            strip_verbatim_prefix("docs/report-?.md"),
            "docs/report-?.md"
        );
        assert_eq!(
            strip_verbatim_prefix(r"\\?\C:\proj\docs\report-?.md"),
            r"C:\proj\docs\report-?.md"
        );
    }
}
