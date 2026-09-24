//! A per-call write-first discipline. No process-global budget and no shell
//! write inference: only successful file mutators can unlock more inspection,
//! and each unlock is bounded — one write never buys unlimited reads.
use crate::tool::ToolContext;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

#[path = "workflow_read_guard_focused.rs"]
mod focused;
#[path = "workflow_read_guard_forbidden.rs"]
mod forbidden;
#[path = "workflow_read_guard_mutators.rs"]
mod mutators;
#[path = "workflow_read_guard_ranges.rs"]
mod ranges;
#[path = "workflow_read_guard_read_only.rs"]
mod read_only;
#[path = "workflow_read_guard_records.rs"]
mod records;
#[path = "workflow_read_guard_run_store.rs"]
mod run_store;
#[path = "workflow_read_guard_settings.rs"]
mod settings;
#[path = "workflow_read_guard_shell.rs"]
mod shell;
#[path = "workflow_read_guard_shell_writes.rs"]
mod shell_writes;
#[path = "workflow_read_guard_targets.rs"]
mod targets;
#[path = "workflow_read_guard_thrash.rs"]
mod thrash;
pub use focused::FocusedTestPlan;
use focused::FocusedTests;
pub use forbidden::{ForbiddenPathScope, scope_forbidden_paths};
pub use mutators::{TreeWideMutator, default_tree_wide_mutators};
pub use read_only::READ_CEILING_MARKER;
use records::{append_record, clip, first_line, record_head};
pub use run_store::{RunStoreScope, current_run_store, scope_run_store};
pub use settings::WorkflowReadGuardSettings;
pub use targets::{DeclaredTargetScope, scope_declared_targets};
pub use thrash::{MAX_NON_WRITING_CALLS_AFTER_WALL, READ_WALL_THRASH_MARKER};

tokio::task_local! { static READ_SET_PATH: PathBuf; }
tokio::task_local! { static FOCUSED_TESTS: FocusedTestPlan; }

/// Record kinds this guard appends to the per-call sidecar beside the read
/// ranges, so the next session of the same branch can be told what this one
/// tried. Read by `archon_workflow::v2::write::session_memory` by these names;
/// a read-range record carries no `kind` and is untouched.
///
/// A refusal: `{"kind":"refusal","call":N,"tool":..,"head":..,"reason":..}`.
pub const REFUSAL_RECORD_KIND: &str = "refusal";
/// A finished or refused call: `{"kind":"tool_call","call":N,"tool":..,"head":..,"status":..}`.
pub const TOOL_CALL_RECORD_KIND: &str = "tool_call";
/// The most characters of a command, path or reason kept in a record. The
/// head is taken from the tool INPUT (command, path, pattern), never from
/// what the tool returned, so file contents never reach the record.
pub const RECORD_HEAD_CHARS: usize = 120;

pub async fn scope_read_set<T>(path: PathBuf, work: impl std::future::Future<Output = T>) -> T {
    READ_SET_PATH.scope(path, work).await
}

/// The focused tests a write call's task declares, for the guard built inside
/// `work` to track. Same task-local shape as the read set: the pipeline
/// constructs the guard per session and cannot be handed the plan directly.
pub async fn scope_focused_tests<T>(
    plan: FocusedTestPlan,
    work: impl std::future::Future<Output = T>,
) -> T {
    FOCUSED_TESTS.scope(plan, work).await
}

/// Command text as the guard compares it: trimmed, whitespace collapsed.
fn normalise_command(command: &str) -> String {
    command.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The inspection shapes both modes count: the inspection tools, and a Bash
/// command the shell classifier recognises as read-only.
fn inspection_call(name: &str, command: &str) -> bool {
    matches!(name, "Read" | "Grep" | "Glob" | "read-own-evidence")
        || (name == "Bash" && shell::inspection(command))
}

#[derive(Debug, Default)]
struct State {
    /// Inspection calls since the allowance was last granted.
    reads: u32,
    /// Inspection calls currently permitted before refusal.
    allowance: u32,
    calls: u64,
    /// Every tool call since the last substantive write; bounds classifier misses.
    calls_since_write: u64,
    writes: u32,
    ranges: BTreeMap<(PathBuf, usize, usize), (String, u64)>,
    focused: Option<FocusedTests>,
    /// The budget has been refused at least once since the last substantive write (Issue-54).
    wall_hit: bool,
    /// Calls counted as thrash since the wall was hit; see `thrash`.
    non_writing_after_wall: u32,
    /// Set once the thrash cutoff is passed: every later call is refused with it.
    terminal: Option<String>,
    /// Inspection calls a read-only guard has admitted (Issue-58); never
    /// reset, since nothing a read-only call does can earn more reading.
    read_only_inspections: u32,
}

/// What the guard enforces for one workflow call (Issue-21).
///
/// The shell admissions — release builds, git history/worktree mutation and
/// tree-wide mutators — protect the canonical checkout every call runs in,
/// so they apply to every call that can run Bash. The read budget, the
/// read-range dedup, the freshness orientation and the focused-test submit
/// nudge exist to make a coder write; a verifier, reviewer or auditor must
/// be able to read as much as it wants. Live, a verifier ran `cargo build
/// --release --bin archon` in the canonical checkout for 25 minutes because
/// no guard was installed for a call without a write tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardMode {
    /// A call with a file-mutating tool: shell admission plus the write-first
    /// read budget and everything that hangs off it.
    WriteCapable,
    /// A call that can only inspect and run Bash: shell admission plus the
    /// inspection ceilings (Issue-58). Every other entry point is a no-op.
    ReadOnly,
}

#[derive(Debug)]
pub struct WorkflowReadGuard {
    mode: GuardMode,
    max_reads: u32,
    reads_per_write: u32,
    allow_release_builds: bool,
    allow_git_mutation: bool,
    allow_tree_wide_mutators: bool,
    tree_wide_mutators: Vec<TreeWideMutator>,
    enforce_declared_targets: bool,
    read_set_path: Option<PathBuf>,
    /// The task's forbidden paths (Issue-30), from the dispatch scope.
    forbidden: Option<ForbiddenPathScope>,
    /// The branch's declared (widened) targets (Issue-64), from the dispatch
    /// scope; `None` when unscoped or switched off by the operator.
    declared: Option<DeclaredTargetScope>,
    /// The run's own record directory, from the dispatch scope; `None` when
    /// the call has no run store to protect.
    run_store: Option<RunStoreScope>,
    /// The read-only ceilings (Issue-58); 0 is off. Unread in write mode.
    read_only_soft_ceiling: u32,
    read_only_hard_ceiling: u32,
    state: Mutex<State>,
}

#[path = "workflow_read_guard_build.rs"]
mod build;

impl WorkflowReadGuard {
    pub fn mode(&self) -> GuardMode {
        self.mode
    }

    fn read_only(&self) -> bool {
        self.mode == GuardMode::ReadOnly
    }

    /// Track the declared focused tests directly, for a guard built outside a
    /// `scope_focused_tests` scope. Ignored by a read-only guard: the submit
    /// nudge exists for the call that owns the files.
    #[must_use]
    pub fn with_focused_tests(self, plan: FocusedTestPlan) -> Self {
        if !self.read_only() {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.focused = FocusedTests::new(plan);
        }
        self
    }

    /// Called at the common tool-dispatch boundary; admission is atomic even
    /// when the model asks for several reads in the same parallel round.
    ///
    /// A refusal is also written to the sidecar, as a `refusal` record and as
    /// a refused `tool_call`, so a session restarted or retried in this
    /// worktree is told not to try the same call again. Live, a retry spent
    /// its first ten minutes re-running the release build and the `git
    /// worktree add` the previous session had already been refused.
    pub fn before_tool(&self, name: &str, input: &Value) -> Option<String> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.calls = state.calls.saturating_add(1);
        state.calls_since_write = state.calls_since_write.saturating_add(1);
        let call = state.calls;
        let refusal = match state.terminal.clone() {
            Some(terminal) => terminal,
            None => {
                let verdict = self.admit(&mut state, name, input);
                thrash::observe(&mut state, name, input, verdict)?
            }
        };
        drop(state);
        let head = record_head(name, input);
        let reason = first_line(&refusal);
        self.remember(json!({
            "kind": REFUSAL_RECORD_KIND, "call": call, "tool": name, "head": head, "reason": reason,
        }));
        self.remember(json!({
            "kind": TOOL_CALL_RECORD_KIND, "call": call, "tool": name, "head": head,
            "status": format!("refused: {reason}"),
        }));
        Some(refusal)
    }

    fn admit(&self, state: &mut State, name: &str, input: &Value) -> Option<String> {
        let command = input.get("command").and_then(Value::as_str).unwrap_or("");
        if name == "Bash" && !self.allow_release_builds && shell::release_build(command) {
            let scope = match self.mode {
                GuardMode::WriteCapable => "this write-capable workflow call",
                GuardMode::ReadOnly => "workflow calls",
            };
            return Some(format!(
                "Release builds are disabled for {scope}. Use cargo check -p <crate> and focused tests; the operator may enable workflow.generated.allow_release_builds."
            ));
        }
        if name == "Bash"
            && !self.allow_git_mutation
            && let Some(verb) = shell::git_mutation(command)
        {
            return Some(format!(
                "git {verb} is refused: git history/worktree mutation is host-owned in workflow runs — the write coordinator commits your files from this worktree. Do not stash, checkout, switch, reset, rebase, merge, cherry-pick, clean, commit or push. To compare against the baseline read-only use `git diff`, `git diff HEAD -- <path>`, `git show HEAD:<path>` or `git status`. The operator may enable workflow.generated.allow_git_mutation."
            ));
        }
        // A formatter or fixer over the whole tree touches files outside the
        // declared targets; each is an undeclared change the patch has to
        // drop (Issue-13). Refusing it here costs one tool call, not a wave.
        if name == "Bash"
            && !self.allow_tree_wide_mutators
            && let Some(refusal) =
                mutators::tree_wide_mutation(&shell::commands(command), &self.tree_wide_mutators)
        {
            return Some(refusal);
        }
        // A write into the run's own record directory is refused wherever it
        // comes from, and in either guard mode: the host writes and parses
        // those files on every stage's input path, so a file of the agent's
        // among them is a failure the whole rest of the run inherits.
        if let Some(refusal) = self.run_store.as_ref().and_then(|r| r.refusal(name, input)) {
            return Some(refusal);
        }
        // A file-mutating call at a path the task forbids is refused before
        // it changes anything (Issue-30); the capture backstop in the write
        // layer catches what a shell edit does around this.
        // A path the branch DECLARES is its own even when the forbidden list
        // also names it — the capture backstop honours the declaration, so
        // the guard must not be stricter for the same path and push the agent
        // to a shell write it cannot see.
        if let Some(refusal) = self.forbidden.as_ref().and_then(|f| f.refusal(name, input))
            && !self
                .declared
                .as_ref()
                .is_some_and(|d| d.declares_call_target(name, input))
        {
            return Some(refusal);
        }
        // A file-mutating call — or a shell write naming its file — at a
        // worktree path outside the declared targets is refused before the
        // gate has to drop it (Issue-64).
        if let Some(refusal) = self.declared.as_ref().and_then(|d| d.refusal(name, input)) {
            return Some(refusal);
        }
        // A read-only call answers to the three shell admissions above and
        // to its own inspection ceilings, never to the budget, the focused
        // submit nudge or the fallback below.
        if self.read_only() {
            return read_only::admit(self, state, name, input);
        }
        let inspection = inspection_call(name, command);
        // Past the grace allowance after every declared focused test passed,
        // inspection and build/test calls are refused with the instruction
        // repeated. Write-class tools are never refused here: the agent may
        // need one last edit before it returns the envelope.
        if let Some(focused) = state.focused.as_mut()
            && focused.complete_at_call.is_some()
        {
            focused.calls_after_complete = focused.calls_after_complete.saturating_add(1);
            if focused.calls_after_complete > u64::from(focused.submit_grace_calls)
                && (inspection || (name == "Bash" && shell::build_or_test(command)))
            {
                return Some(format!(
                    "{} ({} tool calls since; {} were allowed).",
                    focused.submit_instruction(),
                    focused.calls_after_complete,
                    focused.submit_grace_calls,
                ));
            }
        }
        // Hard fallback: a call count far past the phase's budget — 2× max_reads
        // with nothing written, 3× reads_per_write since the last substantive
        // write — refuses inspection-shaped calls the classifier missed. Never
        // matches a write-class tool.
        let fallback = (matches!(name, "Read" | "Grep" | "Glob" | "read-own-evidence")
            || (name == "Bash" && shell::fallback_inspection(command)))
            && if state.writes == 0 {
                state.calls > u64::from(self.max_reads).saturating_mul(2)
            } else {
                state.calls_since_write > u64::from(self.reads_per_write).saturating_mul(3)
            };
        if !inspection && !fallback {
            return None;
        }
        if state.reads >= state.allowance || fallback {
            state.wall_hit = true;
            let mut refusal = if state.writes == 0 {
                format!(
                    "read budget exhausted ({} reads, 0 substantive writes). Write a deliverable file now; each successful substantive Write, Edit, ApplyPatch, NotebookEdit or LargeEditCommit grants {} further reads. Failed, unchanged and whitespace-only writes do not count; Bash alone does not unlock this budget. If the task is already satisfied by the tree — its declared focused checks pass with no edit of yours — stop reading and return status \"noop\" with commands_run and task_coverage evidence instead of writing anything.",
                    state.reads, self.reads_per_write
                )
            } else {
                format!(
                    "read budget exhausted ({} reads since your last substantive write; {} write{} so far). Write or edit a deliverable file now; each successful substantive write grants {} further reads. Failed, unchanged and whitespace-only writes do not count; Bash alone does not unlock this budget.",
                    state.reads,
                    state.writes,
                    if state.writes == 1 { "" } else { "s" },
                    self.reads_per_write
                )
            };
            if fallback && state.writes > 0 {
                refusal.push_str(&format!(
                    " ({} tool calls since your last substantive write)",
                    state.calls_since_write
                ));
            }
            return Some(refusal);
        }
        state.reads += 1;
        None
    }

    /// Observe a finished tool call. A Bash call that exited 0 and contains a
    /// declared focused test command (as a segment of a longer chain, or on
    /// its own) marks that test passed; segment-level exit is not observable,
    /// so the whole call must have exited 0.
    ///
    /// `status` is how the call ended as the caller saw it (`exit 0`,
    /// `exit 101`, `ok`, `error: <first line>`); it is recorded with the
    /// input head so the next session sees what this one ran. It must not
    /// carry tool output.
    pub fn after_tool(&self, name: &str, input: &Value, exit_zero: bool, status: &str) {
        if self.read_only() {
            return;
        }
        let call = self.state.lock().unwrap_or_else(|e| e.into_inner()).calls;
        self.remember(json!({
            "kind": TOOL_CALL_RECORD_KIND, "call": call, "tool": name,
            "head": record_head(name, input), "status": clip(status, RECORD_HEAD_CHARS),
        }));
        if name != "Bash" || !exit_zero {
            return;
        }
        let Some(command) = input.get("command").and_then(Value::as_str) else {
            return;
        };
        let command = normalise_command(command);
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let calls = state.calls;
        let Some(focused) = state.focused.as_mut() else {
            return;
        };
        for (index, declared) in focused.declared.iter().enumerate() {
            if command.contains(declared.as_str()) {
                focused.passed[index] = true;
            }
        }
        if focused.complete_at_call.is_none() && focused.passed.iter().all(|passed| *passed) {
            focused.complete_at_call = Some(calls);
        }
    }

    /// The one-time host turn telling the agent every declared focused test
    /// has passed and it should return the envelope. `None` until then, and
    /// `None` again once it has been handed out.
    pub fn completion_message(&self) -> Option<String> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let focused = state.focused.as_mut()?;
        if focused.complete_at_call.is_none() || focused.announced {
            return None;
        }
        focused.announced = true;
        Some(focused.submit_instruction())
    }

    /// The text the session must end with, once the thrash cutoff is passed
    /// (Issue-54). Read by the subagent runner after each tool round.
    pub fn terminal_failure(&self) -> Option<String> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .terminal
            .clone()
    }
}

impl WorkflowReadGuard {
    /// Best effort: a session record that cannot be written must not fail the
    /// call it describes, unlike a read range, whose loss would silently cost
    /// the retry its orientation.
    fn remember(&self, record: Value) {
        if let Some(sink) = &self.read_set_path {
            let _ = append_record(sink, &record);
        }
    }
}

pub(crate) fn record_write(ctx: &ToolContext, before: &[u8], after: &[u8]) {
    if let Some(guard) = &ctx.workflow_read_guard {
        guard.record_write(before, after);
    }
}

#[cfg(test)]
#[path = "workflow_read_guard_mode_tests.rs"]
mod mode_tests;
#[cfg(test)]
#[path = "workflow_read_guard_read_only_tests.rs"]
mod read_only_tests;
#[cfg(test)]
#[path = "workflow_read_guard_thrash_tests.rs"]
mod thrash_tests;
