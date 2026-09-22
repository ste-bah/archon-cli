//! Issue-77: one command string is one command record.
//!
//! Live, a read-only verification branch landed a verify record whose
//! `commands_run` held a failing test command with no `pre_existing`
//! attribution, then returned its result envelope carrying the SAME command.
//! Expansion appended the landed copy verbatim, so the result held that command
//! twice, the accepted-verdict check named it twice, and the envelope was
//! rejected. The branch then repaired the only copy it owns — the envelope's —
//! with `pre_existing: true` and real evidence, but the copy contributed by the
//! landed record still said false, so exactly one unattributed copy survived.
//! Both rejections shared a repair class, so the bounded repair loop could not
//! earn a further attempt and the branch failed with nothing wrong with the
//! task it was verifying.
//!
//! So landed commands are folded by their command STRING rather than appended.
//! The side already in the list is the newer statement (the envelope, or the
//! newer landing) and keeps kind, status and exit code; the attribution and the
//! evidence text are unioned, so a correction cannot be undone by the stale
//! copy it supersedes.
use crate::WorkflowV2CommandRecord;

/// Fold every entry of `incoming` into `commands` by command string.
pub(crate) fn fold_landed_commands(
    commands: &mut Vec<WorkflowV2CommandRecord>,
    incoming: Vec<WorkflowV2CommandRecord>,
) {
    for command in incoming {
        fold_landed_command(commands, command);
    }
}

/// Fold `incoming` into `commands`, keyed on the command string. The first
/// entry with the same command text absorbs it; anything new is appended, so
/// the order of commands the list already had is never disturbed.
pub(crate) fn fold_landed_command(
    commands: &mut Vec<WorkflowV2CommandRecord>,
    incoming: WorkflowV2CommandRecord,
) {
    let Some(existing) = commands
        .iter_mut()
        .find(|existing| existing.command == incoming.command)
    else {
        commands.push(incoming);
        return;
    };
    // The union is over what the shared predicate accepts, never over the raw
    // flag: downstream honours `pre_existing` only with evidence, and a bare
    // flag on one copy must not launder the other into an attribution.
    existing.pre_existing = evidenced(existing) || evidenced(&incoming);
    if prefer_incoming_summary(&existing.output_summary, &incoming.output_summary) {
        existing.output_summary = incoming.output_summary;
    }
}

fn evidenced(command: &WorkflowV2CommandRecord) -> bool {
    super::verification::is_evidenced_pre_existing_failure(command)
}

/// Keep whichever summary carries the evidence: a captured summary beats a
/// blank or host-synthesized one, and between two captured summaries the longer
/// one is the one that says more.
fn prefer_incoming_summary(existing: &str, incoming: &str) -> bool {
    match (
        super::record_landing::captured(existing),
        super::record_landing::captured(incoming),
    ) {
        (false, true) => true,
        (true, true) => incoming.len() > existing.len(),
        _ => existing.trim().is_empty() && !incoming.trim().is_empty(),
    }
}

#[cfg(test)]
#[path = "record_landing_merge_tests.rs"]
mod tests;
