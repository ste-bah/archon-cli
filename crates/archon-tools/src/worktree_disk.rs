//! What an isolated agent costs on disk (#184 M3).
//!
//! Split from `worktree_manager.rs` to keep it under the 500-line gate.
//!
//! The distinction this module exists to keep visible: a worktree is cheap —
//! it shares `.git` and checks out working files only — while build output is
//! not. On this workspace a cold `target/` is gigabytes. Reporting them as one
//! number would hide the only figure that matters.
//!
//! # Why build output is not reported per agent
//!
//! It used to be. Each agent had a scratch build directory named after it,
//! beside its worktree, and `disk_usage` measured that path. The build-cache
//! lease pool replaced that arrangement: build output now goes to a directory
//! named after the SLOT an agent leases, shared in sequence by every agent that
//! ever holds it (see `build_cache_env`). There is no longer a per-agent build
//! directory to measure, and no honest way to attribute a shared cache to one
//! occupant.
//!
//! The measurement did not fail when the arrangement changed — it kept
//! resolving, to a path nothing creates any more, and returned zero. Zero reads
//! exactly like "this agent never built", so the largest thing on the disk went
//! unreported by a command whose entire job is to report it. Sizes are now
//! taken where the bytes actually are: per worktree for the checkout, and once
//! for the pool.

use std::path::{Path, PathBuf};

use crate::worktree_manager::WorktreeManager;

impl WorktreeManager {
    /// Root of the shared build-cache pool.
    ///
    /// Defined here rather than at the call that installs the pool because two
    /// processes need it and only one of them installs anything: a workflow run
    /// creates the pool, while `/worktrees sizes` runs later, in a process that
    /// never had one, and can only report the cache by reading the directory
    /// off disk. A root chosen at the install site would have left the reporter
    /// either duplicating the path or asking a pool that is always `None`.
    pub fn build_cache_root() -> PathBuf {
        Self::worktrees_dir().join("build-cache")
    }

    /// Where an agent's build output USED to go, before the lease pool.
    ///
    /// Nothing writes here any more, and nothing measures it: the only caller
    /// is the prune path, which removes it if it is still there. A machine that
    /// ran the previous layout is holding gigabytes at this path that no later
    /// code would ever name again, and prune is the one operation that can
    /// honestly reclaim them. Delete this once no such machine is left.
    pub fn legacy_scratch_target_dir(owner_id: &str) -> PathBuf {
        Self::worktrees_dir().join(format!("{owner_id}.target"))
    }

    /// Bytes on disk for `owner_id`'s worktree.
    pub fn disk_usage(owner_id: &str) -> WorktreeDiskUsage {
        WorktreeDiskUsage {
            checkout_bytes: directory_size(&Self::worktrees_dir().join(owner_id)),
        }
    }
}

/// What one isolated agent's checkout is holding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WorktreeDiskUsage {
    /// The checked-out working files.
    pub checkout_bytes: u64,
}

impl WorktreeDiskUsage {
    pub fn total_bytes(self) -> u64 {
        self.checkout_bytes
    }

    /// A short human-facing summary, e.g. `210.4 MB`.
    pub fn describe(self) -> String {
        human_bytes(self.checkout_bytes)
    }
}

/// What the shared build-cache pool is holding, slot by slot.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BuildCacheUsage {
    /// `(slot, bytes)`, ascending by slot.
    pub slots: Vec<(usize, u64)>,
}

impl BuildCacheUsage {
    /// Measure whatever slot directories exist under `root`.
    ///
    /// Discovered from the directory rather than from a pool's configured size,
    /// which is the only way to see a slot left behind by an earlier run with a
    /// larger pool — bytes a size report exists to surface are exactly the ones
    /// nothing is using any more.
    pub fn measure(root: &Path) -> Self {
        let Ok(entries) = std::fs::read_dir(root) else {
            return Self::default();
        };
        let mut slots: Vec<(usize, u64)> = entries
            .flatten()
            .filter_map(|entry| {
                let slot =
                    crate::build_cache_env::lease_slot_of_dir_name(entry.file_name().to_str()?)?;
                Some((slot, directory_size(&entry.path())))
            })
            .collect();
        slots.sort_unstable();
        Self { slots }
    }

    pub fn total_bytes(&self) -> u64 {
        self.slots
            .iter()
            .fold(0u64, |total, (_, bytes)| total.saturating_add(*bytes))
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// A short human-facing summary, e.g. `9.7 GB across 2 build-cache slot(s)`.
    pub fn describe(&self) -> String {
        format!(
            "{} across {} build-cache slot(s)",
            human_bytes(self.total_bytes()),
            self.slots.len()
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
