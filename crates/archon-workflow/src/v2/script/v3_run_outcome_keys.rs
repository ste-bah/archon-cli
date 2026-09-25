//! Task ids and remediation keys as the terminal rule compares them.
//!
//! Every id the script reports is a claim, and its spelling is not the
//! host's: ids are trimmed and matched case-insensitively against the task
//! universe, and a match is replaced by the universe's own spelling. An id the
//! universe does not contain stays as written, so it can match nothing a
//! universe task could.
//!
//! A remediation key is a task id, or — for a finding that names tasks but
//! that no single task may act on — `cross:` followed by the sorted named ids
//! joined with `+`, the key the prelude's cross-task remediation uses.

use std::collections::BTreeSet;

/// Prefix of a cross-task remediation key.
pub const CROSS_TASK_KEY_PREFIX: &str = "cross:";

pub(super) struct TaskKeys<'a> {
    pub(super) universe: &'a BTreeSet<String>,
}

impl TaskKeys<'_> {
    /// The universe's spelling of `raw`, when `raw` is a universe task.
    pub(super) fn task(&self, raw: &str) -> Option<String> {
        let raw = raw.trim();
        self.universe
            .iter()
            .find(|task| task.eq_ignore_ascii_case(raw))
            .cloned()
    }

    fn part(&self, raw: &str) -> String {
        self.task(raw).unwrap_or_else(|| raw.trim().to_string())
    }

    /// The canonical form of a task id or remediation key.
    pub(super) fn key(&self, raw: &str) -> String {
        match raw.trim().strip_prefix(CROSS_TASK_KEY_PREFIX) {
            Some(parts) => cross_key(parts.split('+').map(|part| self.part(part))),
            None => self.part(raw),
        }
    }

    /// The task ids a canonical key stands for.
    pub(super) fn parts(&self, key: &str) -> Vec<String> {
        match key.strip_prefix(CROSS_TASK_KEY_PREFIX) {
            Some(parts) => parts.split('+').map(|part| self.part(part)).collect(),
            None => vec![self.part(key)],
        }
    }
}

/// The cross-task key for a set of task ids: sorted, de-duplicated.
pub fn cross_key(ids: impl IntoIterator<Item = String>) -> String {
    let ids: BTreeSet<String> = ids.into_iter().filter(|id| !id.is_empty()).collect();
    format!(
        "{CROSS_TASK_KEY_PREFIX}{}",
        ids.into_iter().collect::<Vec<_>>().join("+")
    )
}
