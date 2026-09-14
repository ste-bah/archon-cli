//! A per-call write-first discipline. No process-global budget and no shell
//! write inference: only successful file mutators can unlock more inspection,
//! and each unlock is bounded — one write never buys unlimited reads.
use crate::tool::ToolContext;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[path = "workflow_read_guard_shell.rs"]
mod shell;
#[path = "workflow_read_guard_mutators.rs"]
mod mutators;
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

/// What a write agent must see pass before the host tells it to submit.
#[derive(Debug, Clone, Default)]
pub struct FocusedTestPlan {
    /// Declared commands, verbatim; empty means the completion signal is inert.
    pub commands: Vec<String>,
    /// Tool calls admitted after the completion message before inspection and
    /// build/test calls are refused.
    pub submit_grace_calls: u32,
}

impl FocusedTestPlan {
    pub fn new(commands: Vec<String>, submit_grace_calls: u32) -> Self {
        Self { commands, submit_grace_calls }
    }
}

/// Command text as the guard compares it: trimmed, whitespace collapsed.
fn normalise_command(command: &str) -> String {
    command.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Which declared focused tests this session has seen exit 0.
#[derive(Debug)]
struct FocusedTests {
    declared: Vec<String>,
    passed: Vec<bool>,
    submit_grace_calls: u32,
    /// The tool call at which the last declared test passed; set once.
    complete_at_call: Option<u64>,
    /// Whether the completion message has been handed to the runner.
    announced: bool,
    /// Tool calls since completion, against the grace allowance.
    calls_after_complete: u64,
}

impl FocusedTests {
    fn new(plan: FocusedTestPlan) -> Option<Self> {
        let declared: Vec<String> = plan
            .commands
            .iter()
            .map(|command| normalise_command(command))
            .filter(|command| !command.is_empty())
            .collect();
        if declared.is_empty() {
            return None;
        }
        Some(Self {
            passed: vec![false; declared.len()],
            declared,
            submit_grace_calls: plan.submit_grace_calls,
            complete_at_call: None,
            announced: false,
            calls_after_complete: 0,
        })
    }

    fn submit_instruction(&self) -> String {
        format!(
            "All declared focused tests have passed in this session ({n} of {n} at tool call {k}). Return the result envelope now. Further verification is the verifier's job; pre-existing failures outside your target_files are to be reported in residual_gaps, not fixed.",
            n = self.declared.len(),
            k = self.complete_at_call.unwrap_or_default(),
        )
    }
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

#[derive(Debug)]
pub struct WorkflowReadGuard {
    max_reads: u32,
    reads_per_write: u32,
    allow_release_builds: bool,
    allow_git_mutation: bool,
    allow_tree_wide_mutators: bool,
    tree_wide_mutators: Vec<TreeWideMutator>,
    read_set_path: Option<PathBuf>,
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

    pub fn from_settings(settings: &WorkflowReadGuardSettings) -> Self {
        Self {
            max_reads: settings.max_reads_before_first_write,
            reads_per_write: settings.reads_per_write,
            allow_release_builds: settings.allow_release_builds,
            allow_git_mutation: settings.allow_git_mutation,
            allow_tree_wide_mutators: settings.allow_tree_wide_mutators,
            tree_wide_mutators: settings.tree_wide_mutators.clone(),
            read_set_path: READ_SET_PATH.try_with(Clone::clone).ok(),
            state: Mutex::new(State {
                allowance: settings.max_reads_before_first_write,
                focused: FOCUSED_TESTS.try_with(Clone::clone).ok().and_then(FocusedTests::new),
                ..State::default()
            }),
        }
    }

    /// Track the declared focused tests directly, for a guard built outside a
    /// `scope_focused_tests` scope.
    #[must_use]
    pub fn with_focused_tests(self, plan: FocusedTestPlan) -> Self {
        {
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
            return Some("Release builds are disabled for this write-capable workflow call. Use cargo check -p <crate> and focused tests; the operator may enable workflow.generated.allow_release_builds.".into());
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

    pub fn orientation(&self) -> String {
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
        if before
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

/// The part of a tool INPUT worth remembering: the command for Bash, the
/// path or pattern for the inspection tools, the first string field for
/// anything else. Whitespace collapsed and clipped; never tool output.
fn record_head(name: &str, input: &Value) -> String {
    let keys: &[&str] = if name == "Bash" {
        &["command"]
    } else {
        &["file_path", "path", "pattern", "query", "command", "url"]
    };
    let text = keys
        .iter()
        .find_map(|key| input.get(*key).and_then(Value::as_str))
        .or_else(|| {
            input
                .as_object()
                .and_then(|object| object.values().find_map(Value::as_str))
        })
        .unwrap_or("");
    clip(&normalise_command(text), RECORD_HEAD_CHARS)
}

fn first_line(text: &str) -> String {
    clip(text.lines().next().unwrap_or("").trim(), RECORD_HEAD_CHARS)
}

/// At most `chars` characters, marked when cut.
fn clip(text: &str, chars: usize) -> String {
    if text.chars().count() <= chars {
        return text.to_string();
    }
    let mut cut: String = text.chars().take(chars.saturating_sub(1)).collect();
    cut.push('\u{2026}');
    cut
}

pub(crate) fn record_write(ctx: &ToolContext, before: &[u8], after: &[u8]) {
    if let Some(guard) = &ctx.workflow_read_guard {
        guard.record_write(before, after);
    }
}

fn append_record(path: &Path, record: &Value) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let mut bytes = serde_json::to_vec(record)?;
    bytes.push(b'\n');
    file.write_all(&bytes)
}
