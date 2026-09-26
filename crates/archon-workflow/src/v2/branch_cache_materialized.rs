//! Whether a landing's materialized project artifacts still stand where they
//! are verified, as the run left them (Issue-113, under the Issue-108 rule).
//!
//! A declared ignored project artifact a landing copied to its verified
//! location (`patch_apply::materialize`) is in no commit, so the git order
//! `landing::landing_holds` reads cannot place it. The host's own ledger can:
//! every copy is appended there once, with a run-wide sequence, and a later
//! re-persist of the same item's manifest cannot erase it. A destination
//! stands when it holds what the run's LAST copy there left -- this
//! landing's own, or a later landing's of this run, exactly as a tracked path
//! may carry a later landing commit's blob. Anything else there -- an edit, a
//! copy the run never made, a deletion -- refuses. So does a manifest that
//! claims copies the ledger does not hold, and a ledger that cannot be read.
//! Agent data is never read.

use std::collections::BTreeSet;
use std::path::Path;

use crate::write_coordinator::PatchManifest;
use crate::write_coordinator::patch_apply::run_materializations;

/// Why a copy `(stage_id, item_id)` EVER placed -- by the ledger, whatever
/// its current manifest now says -- does not hold what the run's last copy
/// there left, or `Ok` when every one does.
pub(crate) fn landing_copies_hold(
    run_root: &Path,
    stage_id: &str,
    item_id: &str,
    manifest: Option<&PatchManifest>,
) -> Result<(), String> {
    let ledger = run_materializations(run_root)
        .map_err(|error| format!("the materialization ledger is unreadable: {error}"))?;
    let own: Vec<_> = ledger
        .iter()
        .filter(|entry| entry.stage_id == stage_id && entry.item_id == item_id)
        .collect();
    if let Some(manifest) = manifest {
        for (path, receipt) in &manifest.materialized {
            if !own
                .iter()
                .any(|entry| &entry.path == path && &entry.receipt == receipt)
            {
                return Err(format!(
                    "{path}: the manifest records a copy the run's ledger does not"
                ));
            }
        }
    }
    let destinations: BTreeSet<&str> = own
        .iter()
        .map(|entry| entry.receipt.destination.as_str())
        .collect();
    for destination in destinations {
        let last = ledger
            .iter()
            .filter(|entry| entry.receipt.destination == destination)
            .max_by_key(|entry| entry.receipt.sequence)
            .expect("an own entry is at this destination");
        let current = super::landing::current_state(Path::new(destination));
        if current != last.receipt.post_hash {
            return Err(format!(
                "{} at {destination} is {current}, but the run's last materialization there ({}/{}) left {}",
                last.path, last.stage_id, last.item_id, last.receipt.post_hash
            ));
        }
    }
    Ok(())
}

/// [`landing_copies_hold`] for a loaded manifest.
pub(crate) fn materialized_holds(run_root: &Path, manifest: &PatchManifest) -> Result<(), String> {
    landing_copies_hold(
        run_root,
        &manifest.stage_id,
        manifest.item_id.as_str(),
        Some(manifest),
    )
}

/// [`landing_copies_hold`] for `(call_id, item_id)`, manifest or none.
pub(super) fn copies_hold(
    v2_store: &crate::v2::result_store::WorkflowV2ResultStore,
    call_id: &str,
    item_id: &str,
) -> bool {
    let manifest = super::manifest_record(v2_store, call_id, item_id);
    let run_root = v2_store.root().parent().unwrap_or(v2_store.root());
    match landing_copies_hold(run_root, call_id, item_id, manifest.as_ref()) {
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
