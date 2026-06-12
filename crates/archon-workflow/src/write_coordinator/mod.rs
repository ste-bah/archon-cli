//! Write Coordinator — safe parallel implementation fanout (PRD-ARCHON-FINALISATION-012).
//!
//! This module owns the config surface, the runtime feature resolver, and the
//! canonical id aliases shared by every coordinator task. Coordinator behavior
//! (worktree isolation, conflict graph, patch capture/apply) lives in sibling
//! modules added by later tasks.

pub mod config;

use std::path::{Path, PathBuf};

pub use config::WriteCoordinatorConfig;

/// Canonical fan-out item identifier (matches `FanoutItem.id`).
pub type ItemId = String;

/// Canonical wave index within a coordinated implementation fanout.
pub type WaveId = u32;

/// Why the coordinator fell back to serial execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SerialFallbackReason {
    /// `[workflow.write_coordinator] enabled = false`.
    FeatureDisabled,
    /// The canonical target root is not a Git repository.
    NonGitRoot,
    /// The tool runner cannot enforce a workspace boundary (set by later tasks).
    BoundaryUnavailable,
}

/// Resolved runtime state of the Write Coordinator for one run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteCoordinatorRuntime {
    Enabled { canonical_root: PathBuf },
    Disabled { reason: SerialFallbackReason },
}

/// Resolve whether coordinated parallel writes are available for this run.
///
/// Enabled only when the feature flag is on AND the canonical root is a Git
/// repository. Boundary enforcement is checked later, at executor level.
pub fn resolve_write_coordinator_runtime(
    canonical_root: &Path,
    cfg: &WriteCoordinatorConfig,
) -> WriteCoordinatorRuntime {
    if !cfg.enabled {
        return WriteCoordinatorRuntime::Disabled {
            reason: SerialFallbackReason::FeatureDisabled,
        };
    }
    if !is_git_repo(canonical_root) {
        return WriteCoordinatorRuntime::Disabled {
            reason: SerialFallbackReason::NonGitRoot,
        };
    }
    WriteCoordinatorRuntime::Enabled {
        canonical_root: canonical_root.to_path_buf(),
    }
}

/// A `.git` directory (main checkout) or `.git` file (linked worktree) both count.
fn is_git_repo(path: &Path) -> bool {
    path.join(".git").exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_file_and_dir_both_detected() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(!is_git_repo(dir.path()));
        std::fs::write(dir.path().join(".git"), "gitdir: elsewhere").expect("write");
        assert!(is_git_repo(dir.path()));
    }
}
