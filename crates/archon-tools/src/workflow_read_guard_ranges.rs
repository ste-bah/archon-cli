//! The read-range dedup, the read-set sidecar and the write credit of a
//! write-capable guard. A read-only guard retains no ranges and earns no
//! credit: every entry point here is a no-op for it.
use super::{WorkflowReadGuard, records::append_record, thrash};
use crate::tool::ToolContext;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::Path;

impl WorkflowReadGuard {
    /// Empty for a read-only guard, which retains no ranges and has no budget
    /// to refresh within.
    pub fn orientation(&self) -> String {
        if self.read_only() {
            return String::new();
        }
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let ranges = state
            .ranges
            .keys()
            .take(200)
            .map(|(path, offset, limit)| {
                format!("{} offset={offset} limit={limit}", path.display())
            })
            .collect::<Vec<_>>()
            .join("; ");
        format!(
            "Historical read-set orientation (not current file contents): {ranges}. Refresh only needed ranges with force_refresh=true, within the read budget."
        )
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
            let root = ctx
                .working_dir
                .canonicalize()
                .unwrap_or_else(|_| ctx.working_dir.clone());
            let record = json!({"path": path.strip_prefix(&root).unwrap_or(path),
                "offset": offset, "limit": limit, "call": call, "hash": hash});
            append_record(sink, &record).map_err(|error| format!(
                "Failed to retain workflow read-set at {}: {error}. Read content withheld rather than silently losing retry evidence.", sink.display()))?;
        }
        state.ranges.insert(key, (hash, call));
        Ok(None)
    }

    /// Grants the post-write allowance when `after` differs substantively.
    /// [`Self::record_write`] for a write at `path`, which does not count
    /// when it is a report in the run's artifact area outside the call's own
    /// workspace (Issue-124): it is not the branch's deliverable — nothing
    /// lands from there — so it must not lift the read wall. Live, one such
    /// report lifted it and let 500+ identical failing checks run uncounted.
    ///
    /// Only for an isolated worktree branch, and never for a path the host's
    /// boundary names writable (a declared project artifact): a call working
    /// in the project root may be writing its own deliverable there.
    ///
    /// Any write that does count and changed the file — substantive or not,
    /// whitespace included — also clears the repeated-failure counts: the
    /// next run of a check may see a new file. Rewriting identical bytes
    /// changes nothing a check could read, so it clears nothing.
    pub fn record_write_at(&self, path: &Path, before: &[u8], after: &[u8]) {
        let report = self.isolated_write_branch()
            && self
                .run_store
                .as_ref()
                .is_some_and(|scope| scope.is_report_outside_workspace(path))
            && !self.declared_writable(path);
        if report {
            return;
        }
        if !self.read_only() && before != after {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.failing_runs.clear();
        }
        self.record_write(before, after);
    }

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
            thrash::on_substantive_write(&mut state);
        }
    }
}
