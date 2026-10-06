//! The run-base worktree: one detached checkout per commit, under the run's
//! own v2 store, created and removed by exact path only.

use std::path::Path;

fn git(repository_root: &Path, args: &[&std::ffi::OsStr]) -> bool {
    archon_shell::spawn::command("git")
        .arg("-C")
        .arg(repository_root)
        .args(args)
        .output()
        .is_ok_and(|output| output.status.success())
}

pub(super) fn add_worktree(repository_root: &Path, worktree: &Path, commit: &str) -> bool {
    if commit.starts_with('-') {
        return false;
    }
    remove_worktree(repository_root, worktree);
    if let Some(parent) = worktree.parent()
        && std::fs::create_dir_all(parent).is_err()
    {
        return false;
    }
    let added = git(
        repository_root,
        &[
            "worktree".as_ref(),
            "add".as_ref(),
            // A path git still records but no longer finds is re-added.
            "--force".as_ref(),
            "--detach".as_ref(),
            worktree.as_os_str(),
            commit.as_ref(),
        ],
    );
    if !added {
        eprintln!(
            "baseline tests: cannot check out {commit} at {}",
            worktree.display()
        );
    }
    added
}

/// Remove only this worktree; no other worktree's record is touched.
pub(super) fn remove_worktree(repository_root: &Path, worktree: &Path) {
    if worktree.exists() {
        let _ = git(
            repository_root,
            &[
                "worktree".as_ref(),
                "remove".as_ref(),
                "--force".as_ref(),
                worktree.as_os_str(),
            ],
        );
    }
    // A leftover directory git no longer knows (an interrupted run) is the
    // host's own scratch: cleared so the next checkout can be made.
    if worktree.exists() {
        let _ = std::fs::remove_dir_all(worktree);
    }
}
