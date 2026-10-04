//! Whether a write session changed its worktree (Issue 263, round 3,
//! decision C): the measure of progress for a session whose provider
//! connection dropped before it could report.
//!
//! The fingerprint covers what a later capture would keep: staged and
//! unstaged changes to tracked files, and every untracked file with its
//! bytes. `None` when the worktree cannot be read as a git checkout, so two
//! unreadable states never look like a change.

use std::path::Path;

use crate::write_coordinator::worktree_isolation::run_git;

/// A digest of the worktree's uncommitted state.
pub(super) fn fingerprint(root: &Path) -> Option<String> {
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
    for name in untracked
        .stdout
        .split(|byte| *byte == 0)
        .filter(|name| !name.is_empty())
    {
        hasher.update(name);
        hasher.update(b"\0");
        let path = root.join(String::from_utf8_lossy(name).as_ref());
        hasher.update(&std::fs::read(path).unwrap_or_default());
        hasher.update(b"\0");
    }
    Some(hasher.finalize().to_hex().to_string())
}
