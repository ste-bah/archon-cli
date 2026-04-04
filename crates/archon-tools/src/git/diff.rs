use git2::{DiffOptions, Repository};

/// Aggregate diff statistics.
#[derive(Debug, Clone)]
pub struct DiffStats {
    pub files_changed: usize,
    pub insertions: usize,
    pub deletions: usize,
}

/// Produce a unified-diff string for the working tree.
///
/// When `staged` is `true`, diffs the index against HEAD (staged changes).
/// When `staged` is `false`, diffs the working tree against the index (unstaged changes).
pub fn git_diff(repo: &Repository, staged: bool) -> Result<String, String> {
    let diff = if staged {
        let head_tree = head_tree(repo)?;
        repo.diff_tree_to_index(head_tree.as_ref(), None, None)
    } else {
        repo.diff_index_to_workdir(None, Some(DiffOptions::new().include_untracked(false)))
    }
    .map_err(|e| format!("Failed to compute diff: {e}"))?;

    let mut buf = Vec::new();
    diff.print(git2::DiffFormat::Patch, |_delta, _hunk, line| {
        buf.extend_from_slice(line.content());
        true
    })
    .map_err(|e| format!("Failed to print diff: {e}"))?;

    String::from_utf8(buf).map_err(|e| format!("Diff contains invalid UTF-8: {e}"))
}

/// Compute aggregate diff statistics for unstaged working-tree changes.
pub fn git_diff_stats(repo: &Repository) -> Result<DiffStats, String> {
    let diff = repo
        .diff_index_to_workdir(None, Some(DiffOptions::new().include_untracked(false)))
        .map_err(|e| format!("Failed to compute diff: {e}"))?;

    let stats = diff
        .stats()
        .map_err(|e| format!("Failed to get diff stats: {e}"))?;

    Ok(DiffStats {
        files_changed: stats.files_changed(),
        insertions: stats.insertions(),
        deletions: stats.deletions(),
    })
}

/// Helper: resolve HEAD to a tree, returning `None` for an empty repo.
fn head_tree(repo: &Repository) -> Result<Option<git2::Tree<'_>>, String> {
    match repo.head() {
        Ok(reference) => {
            let tree = reference
                .peel_to_tree()
                .map_err(|e| format!("Failed to peel HEAD to tree: {e}"))?;
            Ok(Some(tree))
        }
        Err(e) if e.code() == git2::ErrorCode::UnbornBranch => Ok(None),
        Err(e) => Err(format!("Failed to get HEAD: {e}")),
    }
}
