//! Reading a critic's reply before it is a verdict: what an omitted key
//! means, and how far a refused reply parsed. Split from `fidelity_audit.rs`
//! so that file stays under its size budget.

use serde::Deserialize;

use super::FidelityVerdict;

/// One verdict exactly as the critic wrote it. `weakest_task_id` and
/// `quoted_task_text` are optional only so that their absence is seen, never
/// so that it is filled in: [`checked_verdict`] refuses a false verdict
/// without them.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReplyVerdict {
    pub(super) obligation_id: String,
    pub(super) necessarily_true: bool,
    pub(super) weakest_task_id: Option<String>,
    pub(super) reason: String,
    pub(super) quoted_task_text: Option<String>,
}

/// How far a refused reply parsed before its first error. The order is the
/// parse order, so a reply that parses further compares greater: the JSON
/// shape (by byte offset into the document), then the verdict ids against
/// the cluster, then each verdict in cluster order. Every position is
/// finite — an offset is less than the reply's length, which the provider's
/// output limit bounds, and a verdict index is less than the cluster size —
/// so a critic can make progress only a finite number of times.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReplyPosition {
    /// The document is not the verdict shape at this byte offset.
    Shape(usize),
    /// The shape parsed; its verdict ids repeat or do not match the cluster.
    Ids,
    /// The ids matched; the verdict for the n-th obligation of the cluster
    /// (0-based) was refused.
    Verdict(usize),
}

/// A refused reply: the error the critic is shown, and where it occurred.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FidelityRefusal {
    pub message: String,
    pub position: ReplyPosition,
}

impl std::fmt::Display for FidelityRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// The byte offset of serde_json's 1-based line and its column (bytes into
/// that line) in `text`.
pub(super) fn byte_offset(text: &str, line: usize, column: usize) -> usize {
    let before: usize = text
        .split('\n')
        .take(line.saturating_sub(1))
        .map(|line| line.len() + 1)
        .sum();
    before + column
}

/// Decide what an omitted `weakest_task_id` or `quoted_task_text` means, by
/// the verdict's meaning — never by a silent default.
///
/// A false verdict says a task's allowance lets the obligation stay false;
/// it is checkable only with the task it names and the words it quotes, so
/// either key missing or empty is refused, and the error names the key: the
/// critic is shown this error and can supply it. A true verdict has no
/// loophole to name or quote, so the empty string is its correct value and
/// leaving the key out says the same thing; it is accepted, and the omission
/// is logged at debug level so it stays visible.
pub(super) fn checked_verdict(reply: ReplyVerdict) -> Result<FidelityVerdict, String> {
    let id = &reply.obligation_id;
    let given = |value: &Option<String>| value.as_deref().is_some_and(|v| !v.trim().is_empty());
    if reply.necessarily_true {
        if reply.weakest_task_id.is_none() || reply.quoted_task_text.is_none() {
            tracing::debug!(
                obligation_id = %id,
                "true fidelity verdict omits weakest_task_id or quoted_task_text; a true verdict has none, so it is empty"
            );
        }
    } else if !given(&reply.weakest_task_id) {
        return Err(format!(
            "false verdict for {id} names no weakest task: `weakest_task_id` is missing or empty; a false verdict names the listed task whose allowance grants the loophole"
        ));
    } else if !given(&reply.quoted_task_text) {
        return Err(format!(
            "false verdict for {id} quotes nothing from the task: `quoted_task_text` is missing or empty; a false verdict copies the loophole verbatim from its weakest task"
        ));
    }
    Ok(FidelityVerdict {
        obligation_id: reply.obligation_id,
        necessarily_true: reply.necessarily_true,
        weakest_task_id: reply.weakest_task_id.unwrap_or_default(),
        reason: reply.reason,
        quoted_task_text: reply.quoted_task_text.unwrap_or_default(),
    })
}
