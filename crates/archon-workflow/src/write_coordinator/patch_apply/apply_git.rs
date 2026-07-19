use std::path::Path;

use crate::write_coordinator::worktree_isolation::{IsolationError, run_git};

pub(super) fn apply_patch(
    canonical_root: &Path,
    patch_path: &str,
    changed_files: &[String],
) -> Result<(), IsolationError> {
    match run_git(
        &["apply", "--whitespace=nowarn", patch_path],
        canonical_root,
    ) {
        Ok(output) => Ok(output).map(|_| ()),
        Err(first) if has_staged_targets(canonical_root, changed_files) => Err(first),
        Err(first) => run_git(
            &["apply", "--3way", "--whitespace=nowarn", patch_path],
            canonical_root,
        )
        .map(|_| ())
        .map_err(|second| prefer_apply_error(first, second)),
    }
}

fn has_staged_targets(canonical_root: &Path, changed_files: &[String]) -> bool {
    if changed_files.is_empty() {
        return false;
    }
    let mut args: Vec<&str> = vec!["diff", "--cached", "--name-only", "--"];
    args.extend(changed_files.iter().map(String::as_str));
    run_git(&args, canonical_root)
        .map(|out| !out.stdout.is_empty())
        .unwrap_or(true)
}

fn prefer_apply_error(first: IsolationError, second: IsolationError) -> IsolationError {
    let second_text = second.to_string();
    if second_text.contains("does not match index") {
        first
    } else {
        second
    }
}
