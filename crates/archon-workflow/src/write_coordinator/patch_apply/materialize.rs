//! Declared gitignored project artifacts, placed where they are judged
//! (Issue-113).
//!
//! An ignored declared target never enters the canonical tree
//! (`patch_sidecar`): its bytes ride beside the patch and are archived as a
//! run artifact. That is right for the repository and wrong for a PROJECT
//! artifact. Verifiers are handed `.archon/...` paths under the project root
//! (`project_artifact_stamping`), and acceptance resolves them project-first
//! (`ContractRoots`). So a fix that regenerated one in its worktree landed
//! where neither side reads, and the task could never pass: on wf-0ddadd81 a
//! review fix regenerated three declared ignored deliverables, its manifest
//! archived them, and the verifier judged the stale project copy.
//!
//! At landing, every declared target that is a project artifact, that the
//! host captured as ignored bytes, and that the branch changed against its
//! own baseline, is copied from the host's capture -- the sidecar, never the
//! worktree -- to the path the verification prompts are stamped with. The
//! manifest records the destination, its state before and after, and a
//! run-wide sequence, so a resume can order this run's own copies the way
//! git orders its landing commits (`branch_cache_landing`).
//!
//! Nothing else moves. A repository-rooted ignored path stays a run artifact
//! as `patch_sidecar` requires; only a deliverable the task universe declares,
//! inside a namespace no engine code loads from (`materialize_scope`), is
//! copied; a link is never followed; a destination changed since capture is
//! refused as a stale baseline.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub(crate) use super::materialize_ledger::run_materializations;
use super::materialize_scope::{destination, destination_state, project_root};
use crate::write_coordinator::patch_manifest::{MaterializedDeliverable, PatchManifest};

/// What a destination held before this landing's copy.
#[derive(Debug)]
pub(super) enum Before {
    /// Its bytes, or `None` for nothing there.
    Known(Option<Vec<u8>>),
    /// Already this landing's bytes, left by an interrupted earlier apply;
    /// only the hash of what preceded them (the capture baseline) survives.
    Unknown(String),
}

/// What a landing replaced, so a landing that then fails can put it back,
/// and the receipts it placed, for the ledger once it has landed.
#[derive(Debug, Default)]
pub(super) struct Undo {
    entries: Vec<(PathBuf, Before)>,
    placed: Vec<(String, MaterializedDeliverable)>,
}

impl Undo {
    /// Newest first, every entry attempted. `Err` names each path that could
    /// not be put back: the project root is then NOT what it was, and the
    /// landing must say so (`Failure::attention`).
    pub(super) fn restore(self) -> Result<(), String> {
        let mut failed = Vec::new();
        for (path, before) in self.entries.into_iter().rev() {
            let restored = match before {
                Before::Known(Some(bytes)) => std::fs::write(&path, bytes),
                Before::Known(None) => std::fs::remove_file(&path),
                Before::Unknown(baseline) => Err(std::io::Error::other(format!(
                    "held this landing's bytes from an interrupted apply; its state before ({baseline}) is not recoverable"
                ))),
            };
            if let Err(error) = restored {
                failed.push(format!("{}: {error}", path.display()));
            }
        }
        if failed.is_empty() {
            Ok(())
        } else {
            Err(format!("could not restore {}", failed.join("; ")))
        }
    }
}

/// Why a landing's materialization failed. `attention` is set when copies it
/// made could not be undone: the destination is left changed and a person
/// must look (`PatchManifest::needs_attention`).
#[derive(Debug)]
pub(super) struct Failure {
    pub(super) reason: String,
    pub(super) attention: Option<String>,
}

/// The run-wide order of materializations, read lazily from the ledger:
/// nothing is read unless something is copied. Each landing records its
/// copies before the next is applied, so one read per landing sees them all.
#[derive(Debug, Default)]
struct Sequence(Option<u64>);

impl Sequence {
    fn next(&mut self, run_root: &Path) -> Result<u64, String> {
        let last = match self.0 {
            Some(last) => last,
            None => run_materializations(run_root)?
                .iter()
                .map(|entry| entry.receipt.sequence)
                .max()
                .unwrap_or(0),
        };
        self.0 = Some(last + 1);
        Ok(last + 1)
    }
}

/// Record a decided landing's copies in the run's append-only ledger
/// (`materialize_ledger`). If that fails the copies are undone -- or, if
/// even that fails, flagged -- and the landing fails: a copy the ledger does
/// not hold can never be credited.
pub(super) fn record(
    run_root: &Path,
    manifest: &mut PatchManifest,
    undo: Undo,
) -> Result<(), Failure> {
    let stage = manifest.stage_id.clone();
    let item = manifest.item_id.to_string();
    let Err(error) = super::materialize_ledger::append(run_root, &stage, &item, &undo.placed)
    else {
        return Ok(());
    };
    let placed: Vec<String> = undo.placed.iter().map(|(rel, _)| rel.clone()).collect();
    let attention = undo.restore().err();
    if attention.is_none() {
        manifest.materialized.retain(|rel, _| !placed.contains(rel));
    }
    Err(Failure {
        reason: format!("the materialization ledger could not be written: {error}"),
        attention,
    })
}

/// Copy `manifest`'s changed, declared, ignored project artifacts to their
/// verified location and record each on `manifest.materialized`. On error
/// every copy already made is undone and nothing is recorded -- unless the
/// undo itself failed, when the copies left behind are recorded with the
/// failure.
pub(super) fn materialize(run_root: &Path, manifest: &mut PatchManifest) -> Result<Undo, Failure> {
    let mut undo = Undo::default();
    let mut sequence = Sequence::default();
    let Some(project_root) = project_root(run_root) else {
        return Ok(undo);
    };
    let sidecar = crate::write_coordinator::patch_sidecar::sidecar_dir(&manifest.patch_path);
    let mut placed = BTreeMap::new();
    for rel in manifest.declared_target_files.clone() {
        let outcome =
            place_one(&project_root, &sidecar, manifest, &rel, &mut undo).and_then(|placed| {
                match placed {
                    Some((destination, pre_hash, post_hash)) => Ok(Some(MaterializedDeliverable {
                        destination,
                        pre_hash,
                        post_hash,
                        sequence: sequence.next(run_root)?,
                    })),
                    None => Ok(None),
                }
            });
        match outcome {
            Ok(Some(receipt)) => {
                placed.insert(rel.clone(), receipt);
            }
            Ok(None) => {}
            Err(reason) => {
                let reason = format!("{rel}: {reason}");
                let attention = undo.restore().err();
                if attention.is_some() {
                    manifest.materialized.extend(placed);
                }
                return Err(Failure { reason, attention });
            }
        }
    }
    undo.placed = placed.clone().into_iter().collect();
    manifest.materialized.extend(placed);
    Ok(undo)
}

/// One declared target: `Ok(None)` when it is not this module's to move.
fn place_one(
    project_root: &str,
    sidecar: &Path,
    manifest: &PatchManifest,
    rel: &str,
    undo: &mut Undo,
) -> Result<Option<(String, String, String)>, String> {
    // Only a deliverable the task universe declares -- host-parsed, and
    // exactly what acceptance judges -- is ever placed; any other declared
    // ignored path stays the run artifact it always was.
    if !manifest.materializable.contains(rel) {
        return Ok(None);
    }
    let source = sidecar.join(rel);
    match std::fs::symlink_metadata(&source) {
        Ok(meta) if meta.is_file() => {}
        _ => return Ok(None),
    }
    let Some(destination) = destination(project_root, rel) else {
        return Ok(None);
    };
    let Some(baseline) = manifest.destination_baselines.get(rel) else {
        return Err(format!(
            "no destination baseline was recorded for {} at capture; refusing to overwrite it",
            destination.display()
        ));
    };
    // The capture's own record decides what may be copied. A deletion (or no
    // record) means the sidecar is left over from an earlier capture of this
    // item; bytes the capture did not vouch for are refused outright.
    let captured = manifest.post_hashes.get(rel).map(|hash| hash.trim());
    if matches!(captured, None | Some("" | "deleted" | "absent")) {
        return Ok(None);
    }
    let bytes = std::fs::read(&source).map_err(|error| format!("read capture: {error}"))?;
    let post_hash = blake3::hash(&bytes).to_hex().to_string();
    if captured != Some(post_hash.as_str()) {
        return Err("captured bytes do not match the manifest's post-hash".into());
    }
    // Unchanged against the branch's own baseline: the branch did not
    // produce it, so the verified copy is not the branch's to replace.
    if manifest.pre_hashes.get(rel).map(|hash| hash.trim()) == Some(post_hash.as_str()) {
        return Ok(None);
    }
    // VAL-WC-004 for the destination: it must still be what it was when the
    // branch's deliverable was captured. `pre_hashes` is the REPOSITORY's
    // baseline, not this path's, so the capture records its own.
    let now = destination_state(&destination);
    // Already exactly these bytes -- the copy an apply interrupted before its
    // manifest was persisted made. Placed, not stale; nothing is rewritten.
    if now == post_hash {
        let before = match baseline.as_str() {
            "absent" => Before::Known(None),
            _ => Before::Unknown(baseline.clone()),
        };
        undo.entries.push((destination.clone(), before));
        return Ok(Some((
            destination.display().to_string(),
            baseline.clone(),
            post_hash,
        )));
    }
    if &now != baseline {
        return Err(format!(
            "stale baseline at {}: it changed after this branch's deliverable was captured (was {baseline}, now {now}) and was NOT overwritten; re-read it as it is now and regenerate the deliverable against it",
            destination.display()
        ));
    }
    let before = write_file(Path::new(project_root), &destination, &bytes)
        .map_err(|error| format!("copy to {}: {error}", destination.display()))?;
    let pre_hash = before
        .as_deref()
        .map(|bytes| blake3::hash(bytes).to_hex().to_string())
        .unwrap_or_else(|| "absent".to_string());
    undo.entries
        .push((destination.clone(), Before::Known(before)));
    Ok(Some((
        destination.display().to_string(),
        pre_hash,
        post_hash,
    )))
}

/// Write `bytes` at `destination` by rename, returning what was there. No
/// link on the way is followed ([`refuse_links`]).
fn write_file(root: &Path, destination: &Path, bytes: &[u8]) -> std::io::Result<Option<Vec<u8>>> {
    refuse_links(root, destination)?;
    let before = match std::fs::symlink_metadata(destination) {
        Ok(meta) if meta.is_file() => Some(std::fs::read(destination)?),
        Ok(_) => return Err(std::io::Error::other("exists and is not a regular file")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let parent = destination
        .parent()
        .ok_or_else(|| std::io::Error::other("no parent directory"))?;
    std::fs::create_dir_all(parent)?;
    // Again, now every directory exists: nothing may have become a link.
    refuse_links(root, parent)?;
    let name = destination
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let temporary = parent.join(format!(
        ".{name}.{}.archon-materialize.tmp",
        std::process::id()
    ));
    // `create_new` refuses anything already there, a planted link included.
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    std::io::Write::write_all(&mut file, bytes)?;
    drop(file);
    if let Err(error) = std::fs::rename(&temporary, destination) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    Ok(before)
}

/// Every existing component of `path` below `root` is a real directory or
/// file: a link is refused, never followed.
fn refuse_links(root: &Path, path: &Path) -> std::io::Result<()> {
    let inside = path
        .strip_prefix(root)
        .map_err(|_| std::io::Error::other("destination outside the project root"))?;
    let mut cursor = root.to_path_buf();
    for component in inside.components() {
        cursor.push(component);
        match std::fs::symlink_metadata(&cursor) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(std::io::Error::other(format!(
                    "{} is a symlink",
                    cursor.display()
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// Every concrete deliverable path the task universe declares, spelled as a
/// declared target is: the set a caller hands `apply_wave` as
/// `PatchManifest::materializable`.
pub(crate) fn universe_deliverables(
    universe: &crate::task_universe::WorkflowV2TaskUniverse,
) -> std::collections::BTreeSet<String> {
    universe
        .tasks
        .iter()
        .flat_map(|task| &task.deliverable_contracts)
        .filter_map(|contract| {
            let path = contract.artifact_path.trim().trim_start_matches("./");
            let concrete = !path.is_empty()
                && !Path::new(path).has_root()
                && !path.contains(['*', '<', '{', '$']);
            concrete.then(|| path.to_string())
        })
        .collect()
}

#[cfg(test)]
#[path = "materialize_tests.rs"]
mod tests;
