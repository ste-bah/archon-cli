use crate::v2::verification::gap_is_unowned_path;

/// Did this verification outcome fail AND say what is wrong?
///
/// The failure alone is not enough — a branch can fail for a reason no writer
/// can act on (a transport death, an unreadable input), and dispatching a
/// worktree at that wastes a round. A recorded residual gap is the difference:
/// it names a defect, which is a request for work.
///
/// Reads only status and residual_gaps.
pub(crate) fn failed_with_residual_gaps(outcome: &serde_json::Value) -> bool {
    let result = outcome.get("result").unwrap_or(outcome);
    let status = result
        .get("status")
        .or_else(|| outcome.get("status"))
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    if !matches!(status, "failed" | "needs_review" | "blocked") {
        return false;
    }
    let gaps = result
        .get("residual_gaps")
        .or_else(|| outcome.get("residual_gaps"))
        .and_then(|value| value.as_array())
        // Issue-81: a gap citing only paths no task declares asks for no work.
        .map(|gaps| gaps.iter().filter(|gap| !gap_is_unowned_path(gap)).count())
        .unwrap_or(0);
    gaps > 0
}
