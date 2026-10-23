use std::collections::BTreeSet;

/// Authored passability-judge replies stay pending per affected check.
pub(super) fn authored_passability_pending(
    error: &anyhow::Error,
    ids: &BTreeSet<String>,
) -> Option<(BTreeSet<String>, String)> {
    super::super::judge::JudgeIncomplete::caused(error)
        .filter(|incomplete| incomplete.is_authored_reply())
        .map(|_| {
            (
                ids.clone(),
                format!(
                    "the passability judge returned no usable authored verdict: {error:#}; this check remains pending for judge retry"
                ),
            )
        })
}
