//! Isolated committed-tree copies, using the base snapshot's init/fetch scheme.
//! No live checkout files, references, configuration, or object alternates are
//! shared with the command's checkout.
use std::path::Path;

#[derive(Debug, thiserror::Error)]
enum CopyError {
    #[error("copy storage: {0}")]
    Storage(#[from] std::io::Error),
    #[error("copy operation failed: {0}")]
    Operation(String),
}

fn copy(repository_root: &Path, worktree: &Path, commit: &str) -> Result<(), CopyError> {
    if commit.starts_with('-') {
        return Err(CopyError::Operation(
            "revision starts with an option marker".into(),
        ));
    }
    remove_worktree(repository_root, worktree);
    let parent = worktree
        .parent()
        .ok_or_else(|| CopyError::Operation("copy has no parent".into()))?;
    std::fs::create_dir_all(parent)?;
    let isolation_root = parent.join(uuid::Uuid::new_v4().to_string());
    let isolated = crate::isolated_copy::IsolatedGit::new(&isolation_root)
        .map_err(|error| CopyError::Operation(error.to_string()))?;
    let operation = || {
        let run = |args: &[&std::ffi::OsStr]| {
            isolated
                .run(args)
                .map_err(|error| CopyError::Operation(error.to_string()))
        };
        let format = isolated
            .output(&[
                "-C".as_ref(),
                repository_root.as_os_str(),
                "rev-parse".as_ref(),
                "--show-object-format".as_ref(),
            ])
            .map_err(|error| CopyError::Operation(error.to_string()))?;
        let format_option = format!(
            "--object-format={}",
            String::from_utf8_lossy(&format).trim()
        );
        run(&[
            "init".as_ref(),
            "--quiet".as_ref(),
            format_option.as_ref(),
            format!("--template={}", isolated.template().display()).as_ref(),
            worktree.as_os_str(),
        ])?;
        run(&[
            "-C".as_ref(),
            worktree.as_os_str(),
            "fetch".as_ref(),
            "--no-tags".as_ref(),
            "--depth=1".as_ref(),
            repository_root.as_os_str(),
            commit.as_ref(),
        ])?;
        run(&[
            "-C".as_ref(),
            worktree.as_os_str(),
            "checkout".as_ref(),
            "--quiet".as_ref(),
            "--detach".as_ref(),
            "FETCH_HEAD".as_ref(),
        ])
    };
    let result = operation();
    if let Err(error) = std::fs::remove_dir_all(&isolation_root) {
        tracing::warn!(%error, "copy storage cleanup failed");
    }
    result
}

pub(in crate::v2) fn add_worktree(root: &Path, worktree: &Path, commit: &str) -> bool {
    match copy(root, worktree, commit) {
        Ok(()) => true,
        Err(error) => {
            tracing::warn!(%error, "committed-tree copy unavailable");
            remove_worktree(root, worktree);
            false
        }
    }
}

pub(in crate::v2) fn remove_worktree(_: &Path, worktree: &Path) {
    if worktree.exists()
        && let Err(error) = std::fs::remove_dir_all(worktree)
    {
        tracing::warn!(%error, path = %worktree.display(), "committed-tree copy cleanup failed");
    }
}
