//! The `[workflow.generated]` knobs the guard is built from.
use super::mutators::{TreeWideMutator, default_tree_wide_mutators};

/// Everything `[workflow.generated]` decides about the guard, carried as one
/// value from the config to the session that builds the guard.
///
/// The read budget is per run: an item that must read widely before it can
/// write — a multi-file assignment over an unfamiliar tree — is served by
/// raising `max_reads_before_first_write` in `config.toml`, not by a
/// per-item rule here. The thrash cutoff that follows an exhausted budget
/// (`workflow_read_guard_thrash`) is a constant, not a knob: it bounds a
/// session that has stopped making progress, whatever budget it was given.
#[derive(Debug, Clone)]
pub struct WorkflowReadGuardSettings {
    /// `workflow.generated.max_reads_before_first_write` (default 40).
    pub max_reads_before_first_write: u32,
    /// `workflow.generated.reads_per_write` (default 20).
    pub reads_per_write: u32,
    pub allow_release_builds: bool,
    pub allow_git_mutation: bool,
    /// `workflow.generated.allow_tree_wide_mutators`: lets an unscoped
    /// formatter or fixer run over the whole tree.
    pub allow_tree_wide_mutators: bool,
    /// `workflow.generated.tree_wide_mutators`: the command shapes refused
    /// unless scoped; [`default_tree_wide_mutators`] when unset.
    pub tree_wide_mutators: Vec<TreeWideMutator>,
}

impl Default for WorkflowReadGuardSettings {
    fn default() -> Self {
        Self {
            max_reads_before_first_write: 40,
            reads_per_write: 20,
            allow_release_builds: false,
            allow_git_mutation: false,
            allow_tree_wide_mutators: false,
            tree_wide_mutators: default_tree_wide_mutators(),
        }
    }
}
