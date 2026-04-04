use git2::{Repository, StatusOptions};

/// Summary of a git repository's working-tree and index state.
#[derive(Debug, Clone)]
pub struct GitStatusInfo {
    pub modified: Vec<String>,
    pub staged: Vec<String>,
    pub untracked: Vec<String>,
    pub branch: String,
}

/// Collect the status of the repository into a [`GitStatusInfo`].
pub fn git_status(repo: &Repository) -> Result<GitStatusInfo, String> {
    let branch = super::current_branch(repo).unwrap_or_else(|_| "unknown".to_string());

    let mut opts = StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(false);

    let statuses = repo
        .statuses(Some(&mut opts))
        .map_err(|e| format!("Failed to get status: {e}"))?;

    let mut modified = Vec::new();
    let mut staged = Vec::new();
    let mut untracked = Vec::new();

    for entry in statuses.iter() {
        let path = entry.path().unwrap_or("<invalid utf-8>").to_string();
        let st = entry.status();

        // Index (staged) changes
        if st.intersects(
            git2::Status::INDEX_NEW
                | git2::Status::INDEX_MODIFIED
                | git2::Status::INDEX_DELETED
                | git2::Status::INDEX_RENAMED
                | git2::Status::INDEX_TYPECHANGE,
        ) {
            staged.push(path.clone());
        }

        // Working-tree (unstaged) modifications
        if st.intersects(
            git2::Status::WT_MODIFIED
                | git2::Status::WT_DELETED
                | git2::Status::WT_TYPECHANGE
                | git2::Status::WT_RENAMED,
        ) {
            modified.push(path.clone());
        }

        // Untracked
        if st.contains(git2::Status::WT_NEW) {
            untracked.push(path);
        }
    }

    Ok(GitStatusInfo {
        modified,
        staged,
        untracked,
        branch,
    })
}

/// Format a [`GitStatusInfo`] into a human-readable string.
pub fn format_status(info: &GitStatusInfo) -> String {
    let mut out = format!("On branch {}\n", info.branch);

    if info.staged.is_empty() && info.modified.is_empty() && info.untracked.is_empty() {
        out.push_str("Nothing to commit, working tree clean.\n");
        return out;
    }

    if !info.staged.is_empty() {
        out.push_str("\nChanges to be committed:\n");
        for f in &info.staged {
            out.push_str(&format!("  staged:    {f}\n"));
        }
    }

    if !info.modified.is_empty() {
        out.push_str("\nChanges not staged for commit:\n");
        for f in &info.modified {
            out.push_str(&format!("  modified:  {f}\n"));
        }
    }

    if !info.untracked.is_empty() {
        out.push_str("\nUntracked files:\n");
        for f in &info.untracked {
            out.push_str(&format!("  {f}\n"));
        }
    }

    out
}
