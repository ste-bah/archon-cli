//! A per-call write-first discipline. No process-global budget and no shell
//! write inference: only successful file mutators can unlock more inspection,
//! and each unlock is bounded — one write never buys unlimited reads.
use crate::tool::ToolContext;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[path = "workflow_read_guard_shell.rs"]
mod shell;
#[path = "workflow_read_guard_mutators.rs"]
mod mutators;
#[path = "workflow_read_guard_focused.rs"]
mod focused;
#[path = "workflow_read_guard_records.rs"]
mod records;
#[path = "workflow_read_guard_forbidden.rs"]
mod forbidden;
pub use focused::FocusedTestPlan;
pub use forbidden::{ForbiddenPathScope, scope_forbidden_paths};
use focused::FocusedTests;
use records::{append_record, clip, first_line, record_head};
pub use mutators::{TreeWideMutator, default_tree_wide_mutators};

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
}

/// Everything `[workflow.generated]` decides about the guard, carried as one
/// value from the config to the session that builds the guard.
#[derive(Debug, Clone)]
pub struct WorkflowReadGuardSettings {
    pub max_reads_before_first_write: u32,
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
    /// A call that can only inspect and run Bash: shell admission alone.
    /// Every other entry point is a no-op.
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
    read_set_path: Option<PathBuf>,
    /// The task's forbidden paths (Issue-30), from the dispatch scope.
    forbidden: Option<ForbiddenPathScope>,
    state: Mutex<State>,
}

impl WorkflowReadGuard {
    /// The four original knobs; tree-wide mutators are refused by the default
    /// rules. Use [`Self::from_settings`] to carry the configured rules.
    pub fn new(max_reads_before_first_write: u32, reads_per_write: u32, allow_release_builds: bool, allow_git_mutation: bool) -> Self {
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

    /// The read-only guard: the three shell admissions and nothing else.
    /// Refusals are still recorded in the read-set sidecar, when one is
    /// scoped, so a resumed session is told what this one was refused.
    pub fn shell_only(settings: &WorkflowReadGuardSettings) -> Self {
        Self::with_mode(settings, GuardMode::ReadOnly)
    }

    fn with_mode(settings: &WorkflowReadGuardSettings, mode: GuardMode) -> Self {
        let focused = match mode {
            GuardMode::WriteCapable => {
                FOCUSED_TESTS.try_with(Clone::clone).ok().and_then(FocusedTests::new)
            }
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
            read_set_path: READ_SET_PATH.try_with(Clone::clone).ok(),
            forbidden: forbidden::current(),
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
        let refusal = self.admit(&mut state, name, input)?;
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
            return Some(format!("Release builds are disabled for {scope}. Use cargo check -p <crate> and focused tests; the operator may enable workflow.generated.allow_release_builds."));
        }
        if name == "Bash" && !self.allow_git_mutation && let Some(verb) = shell::git_mutation(command) {
            return Some(format!("git {verb} is refused: git history/worktree mutation is host-owned in workflow runs — the write coordinator commits your files from this worktree. Do not stash, checkout, switch, reset, rebase, merge, cherry-pick, clean, commit or push. To compare against the baseline read-only use `git diff`, `git diff HEAD -- <path>`, `git show HEAD:<path>` or `git status`. The operator may enable workflow.generated.allow_git_mutation."));
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
        // A file-mutating call at a path the task forbids is refused before
        // it changes anything (Issue-30); the capture backstop in the write
        // layer catches what a shell edit does around this.
        if let Some(refusal) = self.forbidden.as_ref().and_then(|f| f.refusal(name, input)) {
            return Some(refusal);
        }
        // A read-only call answers to the three shell admissions above and
        // to nothing below: no budget, no nudge, no fallback.
        if self.read_only() {
            return None;
        }
        let inspection = matches!(name, "Read" | "Grep" | "Glob" | "read-own-evidence")
            || (name == "Bash" && shell::inspection(command));
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
        if !inspection && !fallback { return None; }
        if state.reads >= state.allowance || fallback {
            let mut refusal = if state.writes == 0 {
                format!(
                    "read budget exhausted ({} reads, 0 substantive writes). Write a deliverable file now; each successful substantive Write, Edit, ApplyPatch, NotebookEdit or LargeEditCommit grants {} further reads. Failed, unchanged and whitespace-only writes do not count; Bash alone does not unlock this budget.",
                    state.reads, self.reads_per_write
                )
            } else {
                format!(
                    "read budget exhausted ({} reads since your last substantive write; {} write{} so far). Write or edit a deliverable file now; each successful substantive write grants {} further reads. Failed, unchanged and whitespace-only writes do not count; Bash alone does not unlock this budget.",
                    state.reads, state.writes, if state.writes == 1 { "" } else { "s" }, self.reads_per_write
                )
            };
            if fallback && state.writes > 0 {
                refusal.push_str(&format!(" ({} tool calls since your last substantive write)", state.calls_since_write));
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

    /// Empty for a read-only guard, which retains no ranges and has no budget
    /// to refresh within.
    pub fn orientation(&self) -> String {
        if self.read_only() {
            return String::new();
        }
        let state = self.state.lock().unwrap_or_else(|e|e.into_inner());
        let ranges = state.ranges.keys().take(200).map(|(path,offset,limit)|
            format!("{} offset={offset} limit={limit}",path.display())).collect::<Vec<_>>().join("; ");
        format!("Historical read-set orientation (not current file contents): {ranges}. Refresh only needed ranges with force_refresh=true, within the read budget.")
    }

    /// The tool supplies bytes it really read, not a second host-filesystem
    /// lookup. The key includes the actual range, so a new range is never hidden.
    pub(crate) fn read_result(
        &self,
        ctx: &ToolContext,
        path: &Path,
        offset: usize,
        limit: usize,
        bytes: &[u8],
        force: bool,
    ) -> Result<Option<String>, String> {
        if self.read_only() {
            return Ok(None);
        }
        let hash = format!("{:x}", Sha256::digest(bytes));
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let key = (path.to_path_buf(), offset, limit);
        if !force
            && let Some((old_hash, call)) = state.ranges.get(&key)
            && *old_hash == hash
        {
            return Ok(Some(format!(
                "{} offset={offset} limit={limit}: unchanged since your read at call {call}; content omitted. If earlier content was compacted away, Read the same range with force_refresh=true. This still consumes the read-before-write budget.",
                path.display()
            )));
        }
        let call = state.calls;
        if let Some(sink) = &self.read_set_path {
            // Relative paths survive a retry in a fresh worktree. External
            // artifact paths remain absolute because they do not move.
            let root = ctx.working_dir.canonicalize().unwrap_or_else(|_| ctx.working_dir.clone());
            let record = json!({"path": path.strip_prefix(&root).unwrap_or(path),
                "offset": offset, "limit": limit, "call": call, "hash": hash});
            append_record(sink, &record).map_err(|error| format!(
                "Failed to retain workflow read-set at {}: {error}. Read content withheld rather than silently losing retry evidence.", sink.display()))?;
        }
        state.ranges.insert(key, (hash, call));
        Ok(None)
    }

    /// Grants the post-write allowance when `after` differs substantively.
    pub fn record_write(&self, before: &[u8], after: &[u8]) {
        // Deliberately conservative: ignore whitespace everywhere. This can
        // reject a meaningful whitespace edit but never unlocks on formatting.
        if !self.read_only()
            && before
            .iter()
            .filter(|b| !b.is_ascii_whitespace())
            .ne(after.iter().filter(|b| !b.is_ascii_whitespace()))
        {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.writes = state.writes.saturating_add(1);
            state.reads = 0;
            state.calls_since_write = 0;
            state.allowance = self.reads_per_write;
        }
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
