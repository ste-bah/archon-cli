//! Whether a landing's materialized project artifacts still stand where they
//! are verified, as the run left them (Issue-113, under the Issue-108 rule).
//!
//! A declared ignored project artifact a landing copied to its verified
//! location (`patch_apply::materialize`) is in no commit, so the git order
//! `landing::landing_holds` reads cannot place it. The host's own receipts
//! can: every copy records a run-wide sequence on its manifest. A destination
//! stands when it holds what the run's LAST copy there left -- this landing's
//! own, or a later landing's of this run, exactly as a tracked path may carry
//! a later landing commit's blob. Anything else there -- an edit, a copy the
//! run never made, a deletion -- matches no receipt and refuses. Agent data is
//! never read.
//!
//! A manifest recorded before materialization existed carries no receipts
//! and has nothing here to check: it stands or falls exactly as it did.

use std::path::Path;

use crate::write_coordinator::PatchManifest;
use crate::write_coordinator::patch_apply::run_materializations;

/// Why a materialized destination of `manifest` does not hold what the run's
/// last copy there left, or `Ok` when every one does.
pub(crate) fn materialized_holds(run_root: &Path, manifest: &PatchManifest) -> Result<(), String> {
    if manifest.materialized.is_empty() {
        return Ok(());
    }
    let run = run_materializations(run_root);
    for (path, own) in &manifest.materialized {
        let last = run
            .iter()
            .filter(|entry| {
                entry.receipt.destination == own.destination
                    && entry.receipt.sequence >= own.sequence
            })
            .max_by_key(|entry| entry.receipt.sequence);
        let (expected, by) = match last {
            Some(entry) => (
                &entry.receipt.post_hash,
                format!("{}/{}", entry.stage_id, entry.item_id),
            ),
            None => (
                &own.post_hash,
                format!("{}/{}", manifest.stage_id, manifest.item_id),
            ),
        };
        let current = super::landing::current_state(Path::new(&own.destination));
        if &current != expected {
            return Err(format!(
                "{path} at {} is {current}, but the run's last materialization there ({by}) left {expected}",
                own.destination
            ));
        }
    }
    Ok(())
}

/// `materialized_holds` for the manifest filed for `(call_id, item_id)`:
/// `true` when there is none or it carries no copies.
pub(super) fn copies_hold(
    v2_store: &crate::v2::result_store::WorkflowV2ResultStore,
    call_id: &str,
    item_id: &str,
) -> bool {
    let Some(manifest) = super::manifest_record(v2_store, call_id, item_id) else {
        return true;
    };
    let run_root = v2_store.root().parent().unwrap_or(v2_store.root());
    match materialized_holds(run_root, &manifest) {
        Ok(()) => true,
        Err(reason) => {
            eprintln!("branch reuse: {call_id}/{item_id} does not stand: {reason}");
            false
        }
    }
}

#[cfg(test)]
#[path = "branch_cache_materialized_tests.rs"]
mod tests;
