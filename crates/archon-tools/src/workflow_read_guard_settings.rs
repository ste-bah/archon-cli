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
///
/// The read-only ceilings (`workflow_read_guard_read_only`, Issue-58) bound
/// a call that cannot write at all: its deliverable is its final message, so
/// the only way to make it answer is to stop feeding it more to read.
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
    /// `workflow.generated.enforce_declared_targets` (default true): a
    /// write-capable call is refused a Write/Edit/patch — or a shell write
    /// whose target is syntactically recoverable — at a worktree path outside
    /// the branch's declared (obligation-widened) target set, since the gate
    /// drops that change from the patch anyway (Issue-64). Inert for a call
    /// with no declared targets scoped.
    pub enforce_declared_targets: bool,
    /// `workflow.generated.read_only_soft_call_ceiling` (default 80): from
    /// this many inspection calls on, every inspection result a read-only
    /// call gets carries a one-line nudge to produce the deliverable. 0
    /// disables the nudge.
    pub read_only_soft_call_ceiling: u32,
    /// `workflow.generated.read_only_hard_call_ceiling` (default 120): past
    /// this many inspection calls a read-only call's further inspection is
    /// refused; build and test commands still run. 0 disables the refusal.
    pub read_only_hard_call_ceiling: u32,
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
            enforce_declared_targets: true,
            read_only_soft_call_ceiling: 80,
            read_only_hard_call_ceiling: 120,
        }
    }
}
