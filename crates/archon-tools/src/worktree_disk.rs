//! What an isolated agent costs on disk (#184 M3).
//!
//! Split from `worktree_manager.rs` to keep it under the 500-line gate.
//!
//! The distinction this module exists to keep visible: a worktree is cheap —
//! it shares `.git` and checks out working files only — while an agent's build
//! output is not. On this workspace a cold `target/` is gigabytes. Reporting
//! them as one number would hide the only figure that matters.

use std::path::{Path, PathBuf};

use crate::worktree_manager::WorktreeManager;

impl WorktreeManager {
    /// Where an isolated agent's build output goes.
    ///
    /// Beside the worktree rather than inside it, for the same reason the
    /// ownership lock is: the worktree directory is removed wholesale, so a
    /// build directory inside it would make "remove the tree" and "discard the
    /// build output" the same irreversible step — with no way to keep one while
    /// dropping the other, and no way to measure them apart.
    pub fn scratch_target_dir(owner_id: &str) -> PathBuf {
        Self::worktrees_dir().join(format!("{owner_id}.target"))
    }

    /// Bytes on disk for `owner_id`'s worktree and its build directory.
    ///
    /// Separate figures because they differ by orders of magnitude, and an
    /// operator deciding what to prune needs to see which is which.
    pub fn disk_usage(owner_id: &str) -> WorktreeDiskUsage {
        WorktreeDiskUsage {
            checkout_bytes: directory_size(&Self::worktrees_dir().join(owner_id)),
            build_bytes: directory_size(&Self::scratch_target_dir(owner_id)),
        }
    }
}

/// What one isolated agent is holding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WorktreeDiskUsage {
    /// The checked-out working files.
    pub checkout_bytes: u64,
    /// The scratch build directory, if the agent was allowed to build.
    pub build_bytes: u64,
}

impl WorktreeDiskUsage {
    pub fn total_bytes(self) -> u64 {
        self.checkout_bytes.saturating_add(self.build_bytes)
    }

    /// A short human-facing summary, e.g. `210.4 MB (+ 9.7 GB build)`.
    pub fn describe(self) -> String {
        if self.build_bytes == 0 {
            return human_bytes(self.checkout_bytes);
        }
        format!(
            "{} (+ {} build)",
            human_bytes(self.checkout_bytes),
            human_bytes(self.build_bytes)
        )
    }
}

/// Bytes under `path`, or 0 if it cannot be read.
///
/// Iterative rather than recursive: a `target/` nests deeply enough that
/// recursion is a real stack risk, and this walks whatever is there rather than
/// a bounded shape. Unreadable entries are skipped — a size report that fails
/// because one file is locked is worse than one that is slightly low.
pub fn directory_size(path: &Path) -> u64 {
    let mut total = 0u64;
    let mut pending = vec![path.to_path_buf()];

    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                pending.push(entry.path());
            } else {
                total = total.saturating_add(meta.len());
            }
        }
    }
    total
}

fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
#[path = "worktree_disk_tests.rs"]
mod tests;
