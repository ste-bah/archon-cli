//! How a [`WorkflowReadGuard`] is assembled: the settings it reads, the mode
//! it is built in, and the three dispatch scopes it picks up.
//!
//! Split out of the guard itself only for size. Construction is the one part
//! of the guard that is pure wiring — every field here is copied from
//! settings the operator wrote or from a task-local the host dispatch put in
//! place — while the file next door is the rules. A scope is read ONCE, here,
//! because the guard outlives the task-local: it is built per session and
//! consulted from whatever task each later tool call runs in.

use super::*;

impl WorkflowReadGuard {
    /// The four original knobs; tree-wide mutators are refused by the default
    /// rules. Use [`Self::from_settings`] to carry the configured rules.
    pub fn new(
        max_reads_before_first_write: u32,
        reads_per_write: u32,
        allow_release_builds: bool,
        allow_git_mutation: bool,
    ) -> Self {
        Self::from_settings(&WorkflowReadGuardSettings {
            max_reads_before_first_write,
            reads_per_write,
            allow_release_builds,
            allow_git_mutation,
            ..WorkflowReadGuardSettings::default()
        })
    }

    /// The write-capable guard: shell admission and the read budget.
    pub fn from_settings(settings: &WorkflowReadGuardSettings) -> Self {
        Self::with_mode(settings, GuardMode::WriteCapable)
    }

    /// The read-only guard: the three shell admissions and the inspection
    /// ceilings (Issue-58), nothing else. Refusals are still recorded in the
    /// read-set sidecar, when one is scoped, so a resumed session is told
    /// what this one was refused.
    pub fn shell_only(settings: &WorkflowReadGuardSettings) -> Self {
        Self::with_mode(settings, GuardMode::ReadOnly)
    }

    fn with_mode(settings: &WorkflowReadGuardSettings, mode: GuardMode) -> Self {
        let declared_scope = targets::current();
        let focused = match mode {
            GuardMode::WriteCapable => FOCUSED_TESTS
                .try_with(Clone::clone)
                .ok()
                .and_then(FocusedTests::new),
            GuardMode::ReadOnly => None,
        };
        Self {
            mode,
            max_reads: settings.max_reads_before_first_write,
            reads_per_write: settings.reads_per_write,
            allow_release_builds: settings.allow_release_builds,
            allow_git_mutation: settings.allow_git_mutation,
            allow_tree_wide_mutators: settings.allow_tree_wide_mutators,
            tree_wide_mutators: settings.tree_wide_mutators.clone(),
            enforce_declared_targets: settings.enforce_declared_targets,
            read_set_path: READ_SET_PATH.try_with(Clone::clone).ok(),
            forbidden: forbidden::current(),
            declared: settings
                .enforce_declared_targets
                .then(|| declared_scope.clone())
                .flatten(),
            // Not switchable: the declared-target rule is an efficiency guard
            // the operator may trade away, and this is a boundary around the
            // host's own records that no branch has a reason to cross.
            run_store: run_store::current(),
            isolated_worktree: declared_scope
                .as_ref()
                .is_some_and(DeclaredTargetScope::isolated_worktree),
            boundary: declared_scope
                .as_ref()
                .and_then(|d| d.write_boundary().cloned()),
            worktree_root: declared_scope
                .as_ref()
                .and_then(|d| d.worktree_root().cloned()),
            read_only_soft_ceiling: settings.read_only_soft_call_ceiling,
            read_only_hard_ceiling: settings.read_only_hard_call_ceiling,
            state: Mutex::new(State {
                allowance: settings.max_reads_before_first_write,
                focused,
                ..State::default()
            }),
        }
    }

    /// Judge file-mutating calls against `scope` directly, for a guard built
    /// outside a `scope_forbidden_paths` scope.
    #[must_use]
    pub fn with_forbidden_paths(mut self, scope: ForbiddenPathScope) -> Self {
        self.forbidden = Some(scope).filter(|scope| !scope.is_empty());
        self
    }

    /// Judge file-mutating calls against the declared targets in `scope`,
    /// for a guard built outside a `scope_declared_targets` scope. Ignored
    /// when the settings the guard was built from switch the rule off.
    #[must_use]
    pub fn with_declared_targets(mut self, scope: DeclaredTargetScope) -> Self {
        self.isolated_worktree = scope.isolated_worktree();
        self.boundary = scope.write_boundary().cloned();
        self.worktree_root = scope.worktree_root().cloned();
        if self.enforce_declared_targets {
            self.declared = Some(scope).filter(|scope| !scope.is_inert());
        }
        self
    }

    /// Judge writes against the run directory in `scope`, for a guard built
    /// outside a `scope_run_store` scope.
    #[must_use]
    pub fn with_run_store(mut self, scope: RunStoreScope) -> Self {
        self.run_store = Some(scope).filter(|scope| !scope.is_empty());
        self
    }

    /// Whether this is a write-capable call in its own isolated item
    /// worktree: the one shape whose shell the host bounds at the OS level
    /// (Issue-124, `bash_write_sandbox`).
    pub fn isolated_write_branch(&self) -> bool {
        self.mode == GuardMode::WriteCapable && self.isolated_worktree
    }

    /// Whether the operator let this call mutate git history, which then
    /// needs the repository's shared git directory to be writable.
    pub fn allows_git_mutation(&self) -> bool {
        self.allow_git_mutation
    }

    /// The run-store scope this guard judges by, if any.
    pub fn run_store(&self) -> Option<&RunStoreScope> {
        self.run_store.as_ref()
    }
}
