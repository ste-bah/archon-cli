//! Git change discovery and diff parsing for patch capture.
use super::*;

pub(super) fn run_diff(
    isolated: &Path,
    prefix: &[&str],
    targets: &[String],
) -> Result<Vec<u8>, PatchError> {
    let mut args: Vec<&str> = prefix.to_vec();
    args.extend(targets.iter().map(String::as_str));
    Ok(run_git(&args, isolated)
        .map_err(|e| PatchError::GitDiffFailed {
            stderr: e.to_string(),
        })?
        .stdout)
}

pub(super) fn is_tracked(isolated: &Path, rel: &str) -> bool {
    run_git(&["ls-files", "--error-unmatch", rel], isolated).is_ok()
}

/// Whether `.gitignore` covers `rel` — the paths git will refuse to stage.
pub(super) fn is_ignored(isolated: &Path, rel: &str) -> bool {
    run_git(&["check-ignore", "-q", rel], isolated).is_ok()
}

pub(super) fn validated_workspace_changes(
    isolated: &Path,
    plan: &WritePlan,
) -> Result<Vec<String>, PatchError> {
    let mut paths = workspace_changed_paths(isolated)?;
    // Issue-76: a copy of one of the host's own bookkeeping files is never
    // deliverable. Filtered out HERE, before the ownership check and before it
    // can become a diff target, so it is neither landed nor a reason to refuse
    // the branch. The worktree copy is normally gone by now — the branch
    // runner drops it before any gate reads the worktree — and this is the
    // backstop for every other caller of capture.
    paths.retain(|path| {
        !crate::v2::write::host_internal_artifacts::is_host_internal_artifact_path(path)
    });
    if !plan.workspace_boundary_required {
        return Ok(paths);
    }
    for path in &paths {
        let normalized = normalize_target(path, &plan.canonical_root)
            .map_err(|_| PatchError::UndeclaredWrite { path: path.clone() })?;
        if !path_is_owned(&normalized, plan) {
            return Err(PatchError::UndeclaredWrite { path: path.clone() });
        }
    }
    Ok(paths)
}

pub(super) fn diff_targets(
    isolated: &Path,
    declared_targets: &[NormalizedPath],
    changed_paths: &[String],
) -> Vec<String> {
    if !changed_paths.is_empty() {
        return changed_paths.to_vec();
    }
    declared_targets
        .iter()
        .map(NormalizedPath::as_str)
        .filter(|path| !isolated.join(path).is_dir())
        .collect()
}

pub(crate) fn workspace_changed_paths(isolated: &Path) -> Result<Vec<String>, PatchError> {
    let mut out = Vec::new();
    let diff = run_git(
        &["diff", "--name-only", "--no-renames", "-z", "HEAD", "--"],
        isolated,
    )
    .map_err(|e| PatchError::GitDiffFailed {
        stderr: e.to_string(),
    })?;
    out.extend(split_nul_paths(&diff.stdout));
    let untracked = run_git(
        &["ls-files", "--others", "--exclude-standard", "-z"],
        isolated,
    )
    .map_err(|e| PatchError::GitDiffFailed {
        stderr: e.to_string(),
    })?;
    out.extend(split_nul_paths(&untracked.stdout));
    out.sort();
    out.dedup();
    Ok(out)
}

fn split_nul_paths(bytes: &[u8]) -> impl Iterator<Item = String> + '_ {
    bytes
        .split(|byte| *byte == 0)
        .filter(|raw| !raw.is_empty())
        .map(|raw| String::from_utf8_lossy(raw).into_owned())
}

/// Parse `git diff --name-status -z` output: NUL-separated `STATUS\0path\0`
/// records (`--no-renames` guarantees no `R`/`C` two-path records). Robust for
/// paths containing spaces and for binary files (status, not diff text).
pub(super) fn parse_name_status(bytes: &[u8]) -> (Vec<String>, Vec<String>, Vec<String>) {
    let text = String::from_utf8_lossy(bytes);
    let mut fields = text.split('\0').filter(|s| !s.is_empty());
    let mut changed = Vec::new();
    let mut created = Vec::new();
    let mut deleted = Vec::new();
    while let Some(status) = fields.next() {
        let Some(path) = fields.next() else {
            break;
        };
        let path = path.to_string();
        match status.chars().next() {
            Some('A') => created.push(path.clone()),
            Some('D') => deleted.push(path.clone()),
            _ => {}
        }
        changed.push(path);
    }
    (changed, created, deleted)
}
