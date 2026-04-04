use git2::{BranchType, Repository};

/// Information about a single branch.
#[derive(Debug, Clone)]
pub struct BranchInfo {
    pub name: String,
    pub is_current: bool,
    pub is_remote: bool,
}

/// List all branches (local and remote).
pub fn list_branches(repo: &Repository) -> Result<Vec<BranchInfo>, String> {
    let branches = repo
        .branches(None)
        .map_err(|e| format!("Failed to list branches: {e}"))?;

    let head_ref = repo.head().ok().and_then(|h| h.target());

    let mut result = Vec::new();
    for entry in branches {
        let (branch, branch_type) = entry.map_err(|e| format!("Failed to read branch: {e}"))?;

        let name = branch
            .name()
            .map_err(|e| format!("Invalid branch name: {e}"))?
            .unwrap_or("<unnamed>")
            .to_string();

        let is_remote = branch_type == BranchType::Remote;

        let is_current = if is_remote {
            false
        } else {
            branch
                .get()
                .target()
                .zip(head_ref)
                .map(|(a, b)| a == b)
                .unwrap_or(false)
                && branch.is_head()
        };

        result.push(BranchInfo {
            name,
            is_current,
            is_remote,
        });
    }

    Ok(result)
}

/// Create a new local branch pointing at HEAD.
pub fn create_branch(repo: &Repository, name: &str) -> Result<(), String> {
    let head = repo
        .head()
        .map_err(|e| format!("Failed to get HEAD: {e}"))?;

    let commit = head
        .peel_to_commit()
        .map_err(|e| format!("Failed to peel HEAD to commit: {e}"))?;

    repo.branch(name, &commit, false)
        .map_err(|e| format!("Failed to create branch '{name}': {e}"))?;

    Ok(())
}

/// Switch the working tree to the named branch.
///
/// This performs a checkout and updates HEAD.
pub fn switch_branch(repo: &Repository, name: &str) -> Result<(), String> {
    let refname = format!("refs/heads/{name}");

    let obj = repo
        .revparse_single(&refname)
        .map_err(|e| format!("Branch '{name}' not found: {e}"))?;

    repo.checkout_tree(&obj, None)
        .map_err(|e| format!("Failed to checkout branch '{name}': {e}"))?;

    repo.set_head(&refname)
        .map_err(|e| format!("Failed to set HEAD to '{name}': {e}"))?;

    Ok(())
}
