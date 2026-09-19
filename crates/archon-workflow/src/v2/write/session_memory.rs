//! What the previous session of a write branch tried, for the next one.
//!
//! The resume and restart preambles told a session what its worktree holds
//! and how long it has, and nothing else. Live (wf-c95644e1, agents-4-0),
//! the in-run retry and the session after it each re-ran, inside ten
//! minutes, the exact calls the first session had already had refused —
//! `cargo build --release` (release builds disabled) and `git worktree add`
//! (git mutation) — and redid some 150 `git log --all` calls of archaeology,
//! never learning the allowed alternative the first session had found.
//!
//! The tool guard records every refusal and every finished call in the
//! branch's read-set sidecar (`archon_tools::workflow_read_guard`, records
//! keyed by `kind`); this module reads them back and renders a bounded
//! section for the preamble. Heads come from tool INPUT, never output, so
//! no file contents travel through here.
use std::collections::BTreeSet;
use std::path::Path;

use serde_json::Value;

use crate::WorkflowV2ResultStore;

/// `kind` of a guard refusal record.
const REFUSAL_KIND: &str = "refusal";
/// `kind` of a finished-or-refused call record.
const TOOL_CALL_KIND: &str = "tool_call";

/// Distinct refusals shown, at most.
pub(crate) const MAX_REFUSALS: usize = 20;
/// Recent calls shown when the operator sets nothing
/// (`workflow.generated.resume_memory_calls`).
pub(crate) const DEFAULT_LAST_CALLS: usize = 12;
/// Recent calls shown, at most, whatever the operator sets.
pub(crate) const MAX_LAST_CALLS: usize = 50;
/// Characters per rendered line, at most.
pub(crate) const MAX_LINE_CHARS: usize = 200;

/// The bounded, rendered memory of earlier sessions of one branch or task.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionMemory {
    /// One line per distinct refusal, first occurrence first.
    pub refusals: Vec<String>,
    /// One line per call, most recent last.
    pub last_calls: Vec<String>,
    /// The guard's terminal refusal when it ended the previous session for
    /// thrashing past the read wall (Issue-54): the reason with its count,
    /// as the guard wrote it. Kept apart from `refusals` so the cap on
    /// those — which the ~1,100 distinct refused reads of the live session
    /// would fill many times over — can never drop it.
    pub ended_by_host: Option<String>,
}

impl SessionMemory {
    pub fn is_empty(&self) -> bool {
        self.refusals.is_empty() && self.last_calls.is_empty() && self.ended_by_host.is_none()
    }

    /// The memory of this branch's own earlier sessions: the live sidecar the
    /// guard has been appending to under this call id. This is what an
    /// in-run retry and a mid-attempt restart read.
    pub fn for_branch(store: &WorkflowV2ResultStore, call_id: &str, last_calls: usize) -> Self {
        Self::from_sidecars(
            &[crate::v2::write_read_set::path(store, call_id)],
            last_calls,
        )
    }

    /// The memory of earlier branches, in earlier waves, that owned any of
    /// `task_ids`: refusals from all of them, recent calls from the newest.
    /// This is what a fresh attempt at a task resumed from partial work reads.
    pub fn for_tasks(
        store: &WorkflowV2ResultStore,
        task_ids: &[String],
        last_calls: usize,
    ) -> Self {
        if task_ids.is_empty() {
            return Self::default();
        }
        let mut sidecars: Vec<(std::time::SystemTime, std::path::PathBuf)> = store
            .load_branch_outcomes()
            .unwrap_or_default()
            .into_iter()
            .filter(|outcome| {
                outcome
                    .result
                    .as_ref()
                    .and_then(|result| result.data.get("canonical_task_ids"))
                    .and_then(Value::as_array)
                    .is_some_and(|ids| {
                        ids.iter()
                            .filter_map(Value::as_str)
                            .any(|id| task_ids.iter().any(|target| target == id))
                    })
            })
            .filter_map(|outcome| {
                let path = crate::v2::write_read_set::path(store, &outcome.item_id);
                let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok()?;
                Some((modified, path))
            })
            .collect();
        // Oldest first, so the newest sidecar's calls are the ones kept.
        sidecars.sort();
        sidecars.dedup();
        let paths: Vec<_> = sidecars.into_iter().map(|(_, path)| path).collect();
        Self::from_sidecars(&paths, last_calls)
    }

    /// Refusals are collected across every sidecar (deduplicated, oldest
    /// first); the recent calls are the tail of the LAST sidecar only, since
    /// a call trail spliced from two sessions describes neither.
    fn from_sidecars(paths: &[std::path::PathBuf], last_calls: usize) -> Self {
        let last_calls = last_calls.min(MAX_LAST_CALLS);
        let mut seen = BTreeSet::new();
        let mut refusals = Vec::new();
        let mut calls = Vec::new();
        let mut ended_by_host = None;
        for path in paths {
            calls.clear();
            for record in records(path) {
                match record.get("kind").and_then(Value::as_str) {
                    Some(REFUSAL_KIND) => {
                        let reason = record
                            .get("reason")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .trim();
                        if crate::error::is_read_wall_thrash_text(reason) {
                            // Every call after the cut repeats the same
                            // reason under a different head; one is enough.
                            ended_by_host.get_or_insert_with(|| clip(reason, MAX_LINE_CHARS));
                            continue;
                        }
                        let line = line(&record, "reason");
                        if refusals.len() < MAX_REFUSALS && seen.insert(line.clone()) {
                            refusals.push(line);
                        }
                    }
                    Some(TOOL_CALL_KIND) => calls.push(line(&record, "status")),
                    _ => {}
                }
            }
        }
        let keep = calls.len().saturating_sub(last_calls);
        Self {
            refusals,
            last_calls: calls.split_off(keep),
            ended_by_host,
        }
    }

    /// The preamble section, or `None` when there is nothing to say.
    pub fn render(&self) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        let mut text = String::new();
        if let Some(reason) = &self.ended_by_host {
            text.push_str(&format!(
                "The host ended the previous session ({reason}). Reading past the budget does not \
                 help: write or edit a deliverable file first, then run the declared tests."
            ));
        }
        if !self.refusals.is_empty() {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(
                "The previous session had these tool calls refused by the host — do not retry them:",
            );
            for line in &self.refusals {
                text.push_str("\n  - ");
                text.push_str(line);
            }
        }
        if !self.last_calls.is_empty() {
            let n = self.last_calls.len();
            let calls = if n == 1 { "call" } else { "calls" };
            if text.is_empty() {
                text.push_str(&format!(
                    "The previous session's last {n} tool {calls} (most recent last) were:"
                ));
            } else {
                text.push_str(&format!(
                    "\nIts last {n} tool {calls} (most recent last) were:"
                ));
            }
            for line in &self.last_calls {
                text.push_str("\n  - ");
                text.push_str(line);
            }
        }
        Some(text)
    }
}

/// One rendered line: `Tool \`head\` → detail`, clipped.
fn line(record: &Value, detail_key: &str) -> String {
    let field = |key: &str| record.get(key).and_then(Value::as_str).unwrap_or("").trim();
    clip(
        &format!(
            "{} `{}` → {}",
            field("tool"),
            field("head"),
            field(detail_key)
        ),
        MAX_LINE_CHARS,
    )
}

fn clip(text: &str, chars: usize) -> String {
    if text.chars().count() <= chars {
        return text.to_string();
    }
    let mut cut: String = text.chars().take(chars.saturating_sub(1)).collect();
    cut.push('\u{2026}');
    cut
}

/// Every parseable record in the sidecar; a torn final line is skipped, as
/// the read-set loader skips it.
fn records(path: &Path) -> Vec<Value> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .collect()
}

#[cfg(test)]
#[path = "session_memory_tests.rs"]
mod tests;
