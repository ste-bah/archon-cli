//! A per-call write-first discipline. No process-global budget and no shell
//! write inference: only successful file mutators can unlock more inspection.
use crate::tool::ToolContext;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[path = "workflow_read_guard_shell.rs"]
mod shell;

tokio::task_local! { static READ_SET_PATH: PathBuf; }

pub async fn scope_read_set<T>(path: PathBuf, work: impl std::future::Future<Output = T>) -> T {
    READ_SET_PATH.scope(path, work).await
}

#[derive(Debug, Default)]
struct State {
    reads: u32,
    calls: u64,
    written: bool,
    ranges: BTreeMap<(PathBuf, usize, usize), (String, u64)>,
}

#[derive(Debug)]
pub struct WorkflowReadGuard {
    max_reads: u32,
    allow_release_builds: bool,
    read_set_path: Option<PathBuf>,
    state: Mutex<State>,
}

impl WorkflowReadGuard {
    pub fn new(max_reads_before_first_write: u32, allow_release_builds: bool) -> Self {
        Self {
            max_reads: max_reads_before_first_write,
            allow_release_builds,
            read_set_path: READ_SET_PATH.try_with(Clone::clone).ok(),
            state: Mutex::new(State::default()),
        }
    }

    /// Called at the common tool-dispatch boundary; admission is atomic even
    /// when the model asks for several reads in the same parallel round.
    pub fn before_tool(&self, name: &str, input: &Value) -> Option<String> {
        let command = input.get("command").and_then(Value::as_str).unwrap_or("");
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.calls = state.calls.saturating_add(1);
        if name == "Bash" && !self.allow_release_builds && shell::release_build(command) {
            return Some("Release builds are disabled for this write-capable workflow call. Use cargo check -p <crate> and focused tests; the operator may enable workflow.generated.allow_release_builds.".into());
        }
        let inspection = matches!(name, "Read" | "Grep" | "Glob")
            || (name == "Bash" && shell::inspection(command));
        if state.written { return None; }
        let fallback = state.calls > u64::from(self.max_reads).saturating_mul(2)
            && (matches!(name, "Read" | "Grep" | "Glob")
                || (name == "Bash" && shell::fallback_inspection(command)));
        if !inspection && !fallback { return None; }
        if state.reads >= self.max_reads || fallback {
            return Some(format!(
                "read budget exhausted ({} reads, 0 substantive writes). Write a deliverable file now; reads resume after the first successful substantive Write, Edit, ApplyPatch, NotebookEdit or LargeEditCommit. Failed, unchanged and whitespace-only writes do not count; Bash alone does not unlock this budget.",
                state.reads
            ));
        }
        state.reads += 1;
        None
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

    pub(crate) fn record_write(&self, before: &[u8], after: &[u8]) {
        // Deliberately conservative: ignore whitespace everywhere. This can
        // reject a meaningful whitespace edit but never unlocks on formatting.
        if before
            .iter()
            .filter(|b| !b.is_ascii_whitespace())
            .ne(after.iter().filter(|b| !b.is_ascii_whitespace()))
        {
            self.state.lock().unwrap_or_else(|e| e.into_inner()).written = true;
        }
    }
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
