pub mod branch;
pub mod commit;
pub mod diff;
pub mod pr;
pub mod status;

use std::path::Path;

use git2::Repository;

/// Open a git repository at the given working directory.
pub fn open_repo(working_dir: &Path) -> Result<Repository, String> {
    Repository::open(working_dir).map_err(|e| format!("Failed to open git repository: {e}"))
}

/// Return the name of the current (checked-out) branch.
///
/// Falls back to the short SHA of HEAD when in detached-HEAD state.
pub fn current_branch(repo: &Repository) -> Result<String, String> {
    let head = repo
        .head()
        .map_err(|e| format!("Failed to get HEAD: {e}"))?;

    if head.is_branch()
        && let Some(name) = head.shorthand() {
            return Ok(name.to_string());
        }

    // Detached HEAD — return short hash
    let oid = head
        .target()
        .ok_or_else(|| "HEAD has no target".to_string())?;
    let short = &oid.to_string()[..7.min(oid.to_string().len())];
    Ok(short.to_string())
}
