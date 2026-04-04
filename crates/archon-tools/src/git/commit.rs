use git2::Repository;

/// Stage all modified, deleted, and new files in the working tree.
pub fn stage_all(repo: &Repository) -> Result<(), String> {
    let mut index = repo
        .index()
        .map_err(|e| format!("Failed to get index: {e}"))?;

    index
        .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
        .map_err(|e| format!("Failed to stage files: {e}"))?;

    index
        .write()
        .map_err(|e| format!("Failed to write index: {e}"))?;

    Ok(())
}

/// Create a commit from the current index with the given message.
///
/// Returns the full hex SHA-1 of the new commit.
pub fn commit(repo: &Repository, message: &str) -> Result<String, String> {
    let mut index = repo
        .index()
        .map_err(|e| format!("Failed to get index: {e}"))?;

    let tree_oid = index
        .write_tree()
        .map_err(|e| format!("Failed to write tree: {e}"))?;

    let tree = repo
        .find_tree(tree_oid)
        .map_err(|e| format!("Failed to find tree: {e}"))?;

    let sig = repo
        .signature()
        .map_err(|e| format!("Failed to get default signature: {e}"))?;

    // Resolve parent commit (empty repo has no parent)
    let parent = match repo.head() {
        Ok(head) => {
            let commit = head
                .peel_to_commit()
                .map_err(|e| format!("Failed to peel HEAD: {e}"))?;
            Some(commit)
        }
        Err(e) if e.code() == git2::ErrorCode::UnbornBranch => None,
        Err(e) => return Err(format!("Failed to read HEAD: {e}")),
    };

    let parents: Vec<&git2::Commit<'_>> = parent.iter().collect();

    let oid = repo
        .commit(Some("HEAD"), &sig, &sig, message, &tree, &parents)
        .map_err(|e| format!("Failed to create commit: {e}"))?;

    Ok(oid.to_string())
}

/// Generate a prompt suitable for an LLM to produce a commit message from a diff.
pub fn generate_commit_message_prompt(diff: &str) -> String {
    format!(
        "Based on the following git diff, write a concise and descriptive commit message.\n\
         Follow the Conventional Commits format (e.g., feat:, fix:, refactor:, docs:, test:, chore:).\n\
         The message should have a short summary line (max 72 chars) and optionally a body.\n\
         Only output the commit message, nothing else.\n\n\
         ```diff\n{diff}\n```"
    )
}
