//! A landing that failed after -- or while -- placing project artifacts.
//!
//! Split from `patch_apply.rs` to hold the 500-line ceiling.

use std::path::Path;

use super::{ApplyError, ApplyRecord, ManifestStatus, PatchManifest, materialize, persist_status};

/// A landing whose ignored project artifact could not be placed where it is
/// verified has not landed: fail the item. Copies it could not undo stay
/// recorded, with `needs_attention` saying so.
pub(super) fn fail_materialization(
    run_root: &Path,
    run_id: &str,
    stage_id: &str,
    mut updated: PatchManifest,
    rec: &mut ApplyRecord,
    failure: materialize::Failure,
) -> Result<(), ApplyError> {
    let reason = format!(
        "ignored deliverable materialization failed: {}",
        failure.reason
    );
    if let Some(attention) = &failure.attention {
        flag_attention(&mut updated, attention);
    }
    updated.status = ManifestStatus::Failed {
        reason: reason.clone(),
    };
    let reason = attention_prefixed(&updated, &reason);
    rec.items_failed.push((updated.item_id.clone(), reason));
    persist_status(run_root, run_id, stage_id, &updated.item_id, &updated)
}

/// A failed landing that left the project root changed: recorded on the
/// manifest and said out loud. Its item is failed, never accepted.
pub(super) fn flag_attention(updated: &mut PatchManifest, what: &str) {
    eprintln!(
        "write-coordination: NEEDS ATTENTION {}/{}: {what}",
        updated.stage_id, updated.item_id
    );
    updated.needs_attention = Some(match updated.needs_attention.take() {
        Some(earlier) => format!("{earlier}; {what}"),
        None => what.to_string(),
    });
}

pub(super) fn attention_prefixed(updated: &PatchManifest, reason: &str) -> String {
    match &updated.needs_attention {
        Some(attention) => format!("NEEDS ATTENTION ({attention}): {reason}"),
        None => reason.to_string(),
    }
}
