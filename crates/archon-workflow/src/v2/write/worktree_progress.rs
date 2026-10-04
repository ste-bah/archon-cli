//! Whether a write session changed its worktree (Issue 263, round 3,
//! decision C): the measure of progress for a session whose provider
//! connection dropped before it could report.
//!
//! The fingerprint covers what a later capture would keep: staged and
//! unstaged changes to tracked files, captureable regular untracked files
//! and declared ignored deliverables, under the capture file-size cap. `None` when the worktree cannot be read as a git checkout, so two
//! unreadable states never look like a change.

use std::{collections::BTreeSet, path::Path};

use crate::write_coordinator::worktree_isolation::{read_regular_bounded, run_git};

/// A digest of the worktree's uncommitted state.
pub(super) fn fingerprint(root: &Path, declared: &[String]) -> Option<String> {
    let mut hasher = blake3::Hasher::new();
    for args in [
        &["diff", "--binary"][..],
        &["diff", "--cached", "--binary"][..],
    ] {
        let output = run_git(args, root).ok()?;
        hasher.update(&output.stdout);
        hasher.update(b"\0");
    }
    let untracked = run_git(&["ls-files", "--others", "--exclude-standard", "-z"], root).ok()?;
    // Declared targets include the ignored deliverables patch capture keeps.
    // Undeclared ignored dependencies and noise earn no transport credit.
    let paths: BTreeSet<String> = untracked
        .stdout
        .split(|byte| *byte == 0)
        .filter(|name| !name.is_empty())
        .map(|name| String::from_utf8_lossy(name).into_owned())
        .chain(declared.iter().cloned())
        .collect();
    let cap = crate::write_coordinator::WriteCoordinatorConfig::default().max_file_bytes;
    for name in paths {
        if let Ok(Some(bytes)) = read_regular_bounded(&root.join(&name), cap) {
            hasher.update(name.as_bytes());
            hasher.update(b"\0");
            hasher.update(&bytes);
            hasher.update(b"\0");
        }
    }
    Some(hasher.finalize().to_hex().to_string())
}

#[cfg(test)]
#[path = "worktree_progress_tests.rs"]
mod tests;
