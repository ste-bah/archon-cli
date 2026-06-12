//! Durable `[workflow.write_coordinator]` config block.

use serde::{Deserialize, Serialize};

/// Config surface for the parallel-implementation Write Coordinator.
///
/// All fields default so legacy configs without the block keep working.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriteCoordinatorConfig {
    /// Master switch. Disabled means implementation fanout stays serial.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Keep per-item worktrees after a successful apply (debugging aid).
    #[serde(default)]
    pub retain_success_worktrees: bool,
    /// Keep per-item worktrees after a failed item (default: keep for triage).
    #[serde(default = "default_retain_failed")]
    pub retain_failed_worktrees: bool,
    /// Reject any single item patch larger than this many bytes.
    #[serde(default = "default_max_patch_bytes")]
    pub max_patch_bytes: u64,
    /// Reject any single file in a patch larger than this many bytes.
    #[serde(default = "default_max_file_bytes")]
    pub max_file_bytes: u64,
    /// Fail validation when an implementation fanout item declares no targets.
    #[serde(default = "default_fail_on_undeclared_write")]
    pub fail_on_undeclared_write: bool,
    /// Allow coordination even when the canonical repo has uncommitted changes.
    #[serde(default = "default_allow_dirty")]
    pub allow_dirty_canonical_repo: bool,
}

impl Default for WriteCoordinatorConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            retain_success_worktrees: false,
            retain_failed_worktrees: default_retain_failed(),
            max_patch_bytes: default_max_patch_bytes(),
            max_file_bytes: default_max_file_bytes(),
            fail_on_undeclared_write: default_fail_on_undeclared_write(),
            allow_dirty_canonical_repo: default_allow_dirty(),
        }
    }
}

fn default_enabled() -> bool {
    true
}

fn default_retain_failed() -> bool {
    true
}

fn default_max_patch_bytes() -> u64 {
    10_485_760
}

fn default_max_file_bytes() -> u64 {
    1_048_576
}

fn default_fail_on_undeclared_write() -> bool {
    true
}

fn default_allow_dirty() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_toml_yields_defaults() {
        let cfg: WriteCoordinatorConfig = toml::from_str("").expect("deserializes");
        assert_eq!(cfg, WriteCoordinatorConfig::default());
        assert!(cfg.enabled);
    }

    #[test]
    fn partial_toml_keeps_other_defaults() {
        let cfg: WriteCoordinatorConfig =
            toml::from_str("max_patch_bytes = 1024\n").expect("deserializes");
        assert_eq!(cfg.max_patch_bytes, 1024);
        assert!(cfg.fail_on_undeclared_write);
        assert!(cfg.retain_failed_worktrees);
    }
}
