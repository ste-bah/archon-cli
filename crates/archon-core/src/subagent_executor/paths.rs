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
    subagent_id: &str,
) -> Result<WorktreeInfo, String> {
    WorktreeManager::create_worktree_from_path(source_root, &format!("subagent-{subagent_id}"))
        .map_err(|err| {
            format!(
                "failed to create worktree from {}: {err}",
                source_root.display()
            )
        })
}
