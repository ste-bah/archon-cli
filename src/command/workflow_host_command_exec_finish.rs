//! The end of a call (#297 round 8). Published sources leave staging
//! through the anchor, never by a path, and a cleanup failure after a
//! committed publication keeps the receipt and records the failure: the
//! call's outcome and the residue are both on record.
use super::*;
use crate::command::workflow_host_command_publish::CommandStaging;
use crate::command::workflow_host_staging_pause::StagingPause;
use archon_workflow::PublicationReceiptV1;

/// Removes the published sources from staging, one component at a time
/// without following a link. A source that stays is an audited, sealed copy;
/// it is recorded as residue (never dropped) and the next prepare clears it.
pub(crate) fn remove_published(
    staging: &CommandStaging,
    receipt: &PublicationReceiptV1,
    pause: &StagingPause,
    secrets: &HostSecrets,
) {
    for entry in &receipt.entries {
        let relative = std::path::Path::new(&entry.relative_path);
        if let Err(error) = staging.anchor.remove_file(relative) {
            pause.record_residue(
                &staging.root.join(relative),
                "a published source could not be removed from staging",
                &secrets.text(&error.to_string()),
            );
        }
    }
}

/// The call's result once its staging is sealed (`sealed`). Before a
/// commit, a sealing failure replaces the result. After a commit the
/// receipt is kept: failing now would lose it while the live tree keeps
/// the publication. The failure is recorded as residue; sealing already
/// paused the run when its staging could not be removed.
pub(crate) fn settle_cleanup(
    result: WorkflowResult<HostCommandResult>,
    sealed: WorkflowResult<()>,
    root: &std::path::Path,
    pause: &StagingPause,
    secrets: &HostSecrets,
) -> WorkflowResult<HostCommandResult> {
    match (result, sealed) {
        (result, Ok(())) => result,
        (Ok(published), Err(error)) if published.publication_receipt.is_some() => {
            pause.record_residue(
                root,
                "staging could not be sealed after the publication was committed",
                &secrets.text(&error.to_string()),
            );
            Ok(published)
        }
        (_, Err(error)) => Err(error),
    }
}
