use std::path::{Path, PathBuf};

use archon_tools::worktree_manager::{WorktreeInfo, WorktreeManager};

pub(super) fn resolve_cwd(base: &Path, cwd: Option<&str>) -> Option<PathBuf> {
    cwd.map(|raw| {
        let path = PathBuf::from(raw);
        if path.is_absolute() {
            path
        } else {
            base.join(path)
        }
    })
}

pub(super) fn create_worktree(
    source_root: &Path,
    session_id: &str,
    subagent_id: &str,
) -> Result<WorktreeInfo, String> {
    // The `subagent-` prefix used to be spelled out here and at both cleanup
    // sites; it is now one function, so create and cleanup cannot drift onto
    // different directories (#184 M4).
    let owner_id = archon_tools::worktree_ownership::subagent_owner_key(subagent_id);
    WorktreeManager::create_worktree_from_path(source_root, session_id, &owner_id).map_err(|err| {
        format!(
            "failed to create worktree from {}: {err}",
            source_root.display()
        )
    })
}
