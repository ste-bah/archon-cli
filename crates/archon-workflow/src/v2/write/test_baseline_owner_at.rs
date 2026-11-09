//! Resolve a failure's explicit path in the tree being judged.
use std::path::Path;

pub(crate) fn test_file_at(
    root: &Path,
    commit: Option<&str>,
    _: &str,
    test_id: &str,
) -> Option<String> {
    let path = archon_write_plan::lexical_path::portable(test_id.split("::").next()?);
    if path.is_empty()
        || path.ends_with('/')
        || archon_write_plan::lexical_path::rooted(&path)
        || path.split(['/', '\\']).any(|part| part == "..")
    {
        return None;
    }
    let path = path
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect::<Vec<_>>()
        .join("/");
    if path.is_empty() {
        return None;
    }
    match resolve(root, commit, &path) {
        Ok(file) => file,
        Err(error) => {
            tracing::debug!(%error, "file attribution is unavailable");
            None
        }
    }
}
#[derive(Debug, thiserror::Error)]
enum ResolutionError {
    #[error("the judged commit is not recorded")]
    NoCommit,
    #[error("the recorded path could not be read: {detail}")]
    Path { detail: String },
    #[error("the recorded tree is unavailable: {detail}")]
    Tree { detail: String },
}
fn resolve(
    root: &Path,
    commit: Option<&str>,
    path: &str,
) -> Result<Option<String>, ResolutionError> {
    let commit = commit.ok_or(ResolutionError::NoCommit)?;
    let run = |object: &str, mode: &str| {
        crate::write_coordinator::worktree_isolation::run_git(
            &["cat-file", mode, "--", object],
            root,
        )
    };
    run(&format!("{commit}^{{commit}}"), "-e").map_err(|error| ResolutionError::Tree {
        detail: error.to_string(),
    })?;
    let output = run(&format!("{commit}:{path}"), "-t").map_err(|error| ResolutionError::Path {
        detail: error.to_string(),
    })?;
    Ok((output.stdout.trim_ascii() == b"blob").then(|| path.to_string()))
}
#[cfg(test)]
#[path = "test_baseline_owner_at_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "test_baseline_owner_at_agnostic_proof_tests.rs"]
mod agnostic_proof_tests;
