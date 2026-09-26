//! Whether a recorded remediation landing still stands on the tree as the
//! run left it (Issue-108).
//!
//! # The rule it replaces
//!
//! A replayed remediation write stood only when every path its apply
//! manifest recorded still had exactly that post-state (e7e19bd36). That
//! reads the run's OWN later landings as tampering: round 2 of a unit
//! rewrites the file round 1 landed, so on a resume round 1 no longer
//! "holds", is dispatched afresh on top of round 2's tree, and everything
//! after it follows a different path than the uninterrupted run did. Live
//! that re-ran earlier fixes whose files a later landing had touched.
//!
//! # The rule now
//!
//! A resume must equal the uninterrupted run. Landing R stands when, for
//! every path in its manifest (post-hashes and deleted files alike), the
//! tree holds what the LAST landing of this run that changed that path left
//! there -- R itself, or the end of a chain of landings provably after it.
//! Order is proven from host-written manifests only, by content: the
//! apply-time stale recheck makes a manifest's pre-hash the canonical state
//! it was applied over, so a landed manifest of this run (`Applied` or
//! `IdempotentNoop`, same run id) whose pre-state for the path is R's
//! post-state came after R, and one whose post-state is R's pre-state came
//! before it. Following both directions must place EVERY landing of the run
//! that changed the path; the forward end is the last state.
//!
//! Two candidates for one step (the content repeated), a changing writer
//! the chains cannot place (it changed the path from a state no landing
//! left, so something outside the run's landings changed it in between), or
//! a tree that does not hold the forward end: the order or the state is not
//! proven, and the landing does not stand. Agent data is never read.

use std::collections::BTreeSet;
use std::path::Path;

use crate::v2::result_store::WorkflowV2ResultStore;
use crate::write_coordinator::{ManifestStatus, PatchManifest};

/// Why the landing of `manifest` does not stand on `repository_root`, or
/// `Ok` when every path it recorded holds what the run's last landing there
/// left.
pub fn landing_holds(
    v2_store: &WorkflowV2ResultStore,
    repository_root: &Path,
    manifest: &PatchManifest,
) -> Result<(), String> {
    let run = landed_manifests(v2_store, &manifest.run_id);
    for (path, landed) in recorded_states(manifest) {
        let current = current_state(&repository_root.join(&path));
        let last = last_state(&run, manifest, &path, &landed)?;
        if !same(&last, &current) {
            return Err(format!(
                "{path} is {current}, but the run's last landing there left {last}"
            ));
        }
    }
    Ok(())
}

/// Every path a manifest recorded and the state it left there.
fn recorded_states(manifest: &PatchManifest) -> Vec<(String, String)> {
    let mut states: Vec<(String, String)> = manifest
        .post_hashes
        .iter()
        .map(|(path, hash)| (path.clone(), hash.clone()))
        .collect();
    for path in &manifest.deleted_files {
        if !manifest.post_hashes.contains_key(path) {
            states.push((path.clone(), "deleted".to_string()));
        }
    }
    states
}

/// The state the run's LAST landing on `path` left: the forward end of the
/// chain from `origin`, once every landing that changed the path is placed
/// on it or on the chain back from `origin`.
fn last_state(
    run: &[PatchManifest],
    origin: &PatchManifest,
    path: &str,
    landed: &str,
) -> Result<String, String> {
    let writers: Vec<&PatchManifest> = run
        .iter()
        .filter(|candidate| !same_landing(candidate, origin) && changed(candidate, path))
        .collect();
    if writers.is_empty() {
        return Ok(landed.to_string());
    }
    let name = |m: &PatchManifest| format!("{}/{}", m.stage_id, m.item_id);
    let mut placed: BTreeSet<usize> = BTreeSet::new();
    let step = |state: &str, forward: bool, placed: &mut BTreeSet<usize>| {
        let next: Vec<usize> = (0..writers.len())
            .filter(|index| !placed.contains(index))
            .filter(|index| {
                let end = if forward {
                    writers[*index].pre_hashes.get(path).cloned()
                } else {
                    post_state(writers[*index], path)
                };
                end.is_some_and(|end| same(&end, state))
            })
            .collect();
        match next.as_slice() {
            [] => Ok(None),
            [only] => {
                placed.insert(*only);
                Ok(Some(*only))
            }
            _ => Err(format!(
                "{path}: {} landings share the state {state}; their order is not proven",
                next.len()
            )),
        }
    };
    let mut last = landed.to_string();
    while let Some(index) = step(&last, true, &mut placed)? {
        let Some(post) = post_state(writers[index], path) else {
            return Err(format!(
                "{path}: {} recorded no post-state",
                name(writers[index])
            ));
        };
        last = post;
    }
    let mut before = origin.pre_hashes.get(path).cloned();
    while let Some(state) = before {
        before = step(&state, false, &mut placed)?
            .and_then(|index| writers[index].pre_hashes.get(path).cloned());
    }
    if let Some(stray) = (0..writers.len()).find(|index| !placed.contains(index)) {
        return Err(format!(
            "{path}: {} changed it from a state no landing of this run left",
            name(writers[stray])
        ));
    }
    Ok(last)
}

/// Every landed manifest of the run under the host's write-coordination
/// directory (the parent of the v2 store root, as `manifest_record` reads
/// it). An unreadable file is skipped: it can only make a chain shorter.
fn landed_manifests(v2_store: &WorkflowV2ResultStore, run_id: &str) -> Vec<PatchManifest> {
    let run_root = v2_store.root().parent().unwrap_or(v2_store.root());
    let Ok(stages) = std::fs::read_dir(run_root.join("write-coordination").join("stages")) else {
        return Vec::new();
    };
    let mut manifests = Vec::new();
    for stage in stages.flatten() {
        let Ok(entries) = std::fs::read_dir(stage.path().join("manifests")) else {
            continue;
        };
        for entry in entries.flatten() {
            let Some(manifest) = std::fs::read(entry.path())
                .ok()
                .and_then(|bytes| serde_json::from_slice::<PatchManifest>(&bytes).ok())
            else {
                continue;
            };
            if manifest.run_id == run_id
                && matches!(
                    manifest.status,
                    ManifestStatus::Applied | ManifestStatus::IdempotentNoop
                )
            {
                manifests.push(manifest);
            }
        }
    }
    manifests.sort_by(|a, b| (&a.stage_id, &a.item_id).cmp(&(&b.stage_id, &b.item_id)));
    manifests
}

fn same_landing(left: &PatchManifest, right: &PatchManifest) -> bool {
    left.stage_id == right.stage_id && left.item_id == right.item_id
}

/// Whether a landing changed `path`: it wrote it (changed, created or
/// deleted) and left it in another state than it found it.
fn changed(manifest: &PatchManifest, path: &str) -> bool {
    let wrote = [
        &manifest.changed_files,
        &manifest.created_files,
        &manifest.deleted_files,
    ]
    .iter()
    .any(|paths| paths.iter().any(|written| written == path));
    wrote
        && match (manifest.pre_hashes.get(path), post_state(manifest, path)) {
            (Some(pre), Some(post)) => !same(pre, &post),
            _ => true,
        }
}

fn post_state(manifest: &PatchManifest, path: &str) -> Option<String> {
    manifest.post_hashes.get(path).cloned().or_else(|| {
        manifest
            .deleted_files
            .iter()
            .any(|deleted| deleted == path)
            .then(|| "deleted".to_string())
    })
}

/// Two recorded states agree: a pre-image's `absent` and a post-image's
/// `deleted` both mean no file.
fn same(left: &str, right: &str) -> bool {
    let norm = |state: &str| if state == "absent" { "deleted" } else { state }.to_string();
    norm(left) == norm(right)
}

/// A path's state as a manifest records it: its content hash, or `deleted`
/// only when nothing is there. A directory or an unreadable file matches no
/// recorded state.
pub(super) fn current_state(path: &Path) -> String {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => "deleted".to_string(),
        Err(_) => "<unreadable>".to_string(),
        Ok(meta) if !meta.is_file() => "<not a file>".to_string(),
        Ok(_) => crate::write_coordinator::patch_apply::hash_file(path)
            .unwrap_or_else(|| "<unreadable>".to_string()),
    }
}

#[cfg(test)]
#[path = "branch_cache_landing_tests.rs"]
mod tests;
