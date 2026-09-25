//! Why the attempt that left a partial did not finish (Issue-20).
//!
//! A partial is captured whenever a branch ends without a manifest and without
//! acceptance: a host timeout, but just as often a gate REJECTION. The resume
//! preamble used to say "ran out of time" for both, so an agent re-dispatched
//! over a rejected partial was never told what the rejection was and repeated
//! the omission exactly. Live: a coder rejected by the repository audit for an
//! undeclared test file was told it had run out of time, and submitted the same
//! incomplete dispositions again. The origin recorded here carries the verdict
//! (status, summary, residual gaps) so the next attempt is told what to fix.
use serde::{Deserialize, Serialize};

use super::super::errors::truncate_for_result;
use super::PartialWork;
use crate::v2::{WorkflowV2Result, WorkflowV2Status};

/// Sizes that keep an origin readable inside a prompt and a sidecar.
pub(crate) const MAX_SUMMARY_CHARS: usize = 2048;
pub(crate) const MAX_GAP_DESCRIPTION_CHARS: usize = 1024;
pub(crate) const MAX_GAPS: usize = 20;

/// The gap id prefix every host interruption result carries
/// (`errors::write_branch_interrupted_result`): a timeout, a spent call
/// budget, a host resource the branch could not take. It is a key the stall
/// path and `resume` already match on, so it is the stable signal here too.
const INTERRUPTION_GAP_PREFIX: &str = "write_branch_timeout_";
/// The summary the same constructor writes for a host timeout.
const INTERRUPTION_SUMMARY_MARKER: &str = "timed out before returning usable output";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartialOriginGap {
    pub id: String,
    #[serde(default)]
    pub severity: String,
    pub description: String,
}

/// The verdict on the attempt whose work the partial holds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartialOrigin {
    /// The `WorkflowV2Status` the partial was captured from, in its wire
    /// spelling (`needs_review`, `failed`, ...).
    pub status: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub residual_gaps: Vec<PartialOriginGap>,
}

pub(crate) fn status_wire_name(status: WorkflowV2Status) -> String {
    serde_json::to_value(status)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| format!("{status:?}").to_ascii_lowercase())
}

impl PartialOrigin {
    /// The origin a captured partial gets from the branch result at hand.
    pub(crate) fn from_result(result: &WorkflowV2Result) -> Self {
        Self {
            status: status_wire_name(result.status),
            summary: truncate_for_result(result.summary.trim(), MAX_SUMMARY_CHARS),
            residual_gaps: result
                .residual_gaps
                .iter()
                .take(MAX_GAPS)
                .map(|gap| PartialOriginGap {
                    id: gap.id.clone(),
                    severity: gap.severity.clone().unwrap_or_default(),
                    description: truncate_for_result(
                        gap.description.trim(),
                        MAX_GAP_DESCRIPTION_CHARS,
                    ),
                })
                .collect(),
        }
    }

    /// The attempt was STOPPED by the host rather than judged: the same class
    /// `worktree_branch_retry::timed_out_with_work_unjudged` keys on, read
    /// from what the origin carries. Every interruption result is built by
    /// `write_branch_interrupted_result`, whose gap id is the marker; its
    /// summary wording is the fallback for a record whose gaps were trimmed.
    /// Deliberately NOT a bare "timed out" match: an agent's own verdict may
    /// say its tests timed out, and that is a verdict to pass on, not a cut.
    pub(crate) fn is_timeout(&self) -> bool {
        let summary = self.summary.to_ascii_lowercase();
        self.residual_gaps
            .iter()
            .any(|gap| gap.id.starts_with(INTERRUPTION_GAP_PREFIX))
            || summary.contains(INTERRUPTION_SUMMARY_MARKER)
            || summary.contains(super::super::errors::CALL_TIME_BUDGET_EXHAUSTED)
    }

    /// The host cut the attempt for inactivity rather than at its wall clock.
    pub(crate) fn is_stall(&self) -> bool {
        self.summary
            .contains(super::super::errors::STALL_SUMMARY_MARKER)
    }

    /// The verdict sentence for a resume over a judged (not timed-out)
    /// attempt, ending where the file list continues.
    fn verdict(&self) -> String {
        let summary = self.summary.trim().trim_end_matches('.');
        let mut text = format!(
            "A previous attempt at this task was not accepted (status: {})",
            self.status
        );
        if !summary.is_empty() {
            text.push_str(": ");
            text.push_str(summary);
        }
        text.push('.');
        text
    }

    /// The gaps the previous attempt left, one per line, or nothing when the
    /// verdict named none.
    fn gaps_section(&self) -> Option<String> {
        if self.residual_gaps.is_empty() {
            return None;
        }
        let lines: Vec<String> = self
            .residual_gaps
            .iter()
            .map(|gap| {
                let severity = if gap.severity.is_empty() {
                    "gap"
                } else {
                    gap.severity.as_str()
                };
                format!("- [{severity}] {}: {}", gap.id, gap.description.trim())
            })
            .collect();
        Some(format!(
            "Its unresolved gaps, which you must fix:\n{}",
            lines.join("\n")
        ))
    }
}

/// The sentence(s) the host preamble renders for a resumed partial.
///
/// `same_attempt` is a session restarted mid-attempt in its own worktree; the
/// work is the agent's own. Otherwise an earlier attempt left it, and the agent
/// is told how that attempt ended: judged and not accepted (with what was
/// missing), or stopped by the host before any verdict.
pub(crate) fn resumed_sentence(partial: &PartialWork, same_attempt: bool) -> String {
    let judged = partial
        .origin
        .as_ref()
        .filter(|origin| !origin.is_timeout());
    let opening = if same_attempt {
        "This is the same attempt, restarted after the model connection ended; the workspace is exactly as you left it.".to_string()
    } else if let Some(origin) = judged {
        origin.verdict()
    } else if partial.origin.as_ref().is_some_and(PartialOrigin::is_stall) {
        "A previous attempt at this task stalled — no model output or tool activity for the host's inactivity bound — and was cut before finishing.".to_string()
    } else {
        "A previous attempt at this task ran out of time before finishing.".to_string()
    };
    let work = format!(
        "Its uncommitted work ({} file(s)) has been applied to this workspace: {}. Continue from that work; do not start over, and do not discard it unless it is wrong.",
        partial.files.len(),
        partial.files.join(", ")
    );
    // The gap list is line-oriented, so it sits on its own lines between the
    // verdict and the file sentence rather than running into either.
    match judged.and_then(PartialOrigin::gaps_section) {
        Some(gaps) => format!("{opening}\n{gaps}\n{work}"),
        None => format!("{opening} {work}"),
    }
}
