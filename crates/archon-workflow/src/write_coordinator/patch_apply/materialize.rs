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
//! as `patch_sidecar` requires; an undeclared path is never copied; a path
//! into the engine's own run store, or through a symlink, is refused.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::write_coordinator::patch_manifest::{
    ManifestStatus, MaterializedDeliverable, PatchManifest,
};

/// What a landing replaced, so a landing that then fails can put it back.
#[derive(Debug, Default)]
pub(super) struct Undo(Vec<(PathBuf, Option<Vec<u8>>)>);

impl Undo {
    /// Best effort, newest first. A path a restore cannot write is left as
    /// the failed landing left it; the manifest records the failure.
    pub(super) fn restore(self) {
        for (path, before) in self.0.into_iter().rev() {
            let _ = match before {
                Some(bytes) => std::fs::write(&path, bytes),
                None => std::fs::remove_file(&path),
            };
        }
    }
}

/// The run-wide order of materializations, read lazily from the run's own
/// receipts: nothing is counted unless something is copied. Each landing
/// persists its manifest before the next one is applied, so a fresh read per
/// landing already sees every earlier receipt.
#[derive(Debug, Default)]
struct Sequence(Option<u64>);

impl Sequence {
    fn next(&mut self, run_root: &Path) -> u64 {
        let last = *self.0.get_or_insert_with(|| {
            run_materializations(run_root)
                .iter()
                .map(|entry| entry.receipt.sequence)
                .max()
                .unwrap_or(0)
        });
        self.0 = Some(last + 1);
        last + 1
    }
}

/// Copy `manifest`'s changed, declared, ignored project artifacts to their
/// verified location and record each on `manifest.materialized`. On error
/// nothing is recorded and every copy already made is undone.
pub(super) fn materialize(run_root: &Path, manifest: &mut PatchManifest) -> Result<Undo, String> {
    let mut undo = Undo::default();
    let mut sequence = Sequence::default();
    let Some(project_root) = project_root(run_root) else {
        return Ok(undo);
    };
    let sidecar = crate::write_coordinator::patch_sidecar::sidecar_dir(&manifest.patch_path);
    let mut placed = BTreeMap::new();
    for rel in &manifest.declared_target_files {
        match place_one(&project_root, &sidecar, manifest, rel, &mut undo) {
            Ok(Some((destination, pre_hash, post_hash))) => {
                let receipt = MaterializedDeliverable {
                    destination,
                    pre_hash,
                    post_hash,
                    sequence: sequence.next(run_root),
                };
                placed.insert(rel.clone(), receipt);
            }
            Ok(None) => {}
            Err(reason) => {
                undo.restore();
                return Err(format!("{rel}: {reason}"));
            }
        }
    }
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
    let before = write_file(Path::new(project_root), &destination, &bytes)
        .map_err(|error| format!("copy to {}: {error}", destination.display()))?;
    let pre_hash = before
        .as_deref()
        .map(|bytes| blake3::hash(bytes).to_hex().to_string())
        .unwrap_or_else(|| "absent".to_string());
    undo.0.push((destination.clone(), before));
    Ok(Some((
        destination.display().to_string(),
        pre_hash,
        post_hash,
    )))
}

/// The project root verifiers are stamped against: the one
/// `project_artifact_context_from_v2_root` gives the host dispatch.
fn project_root(run_root: &Path) -> Option<String> {
    crate::v2::project_artifacts::project_artifact_context_from_v2_root(&run_root.join("v2"))
        .project_root
        .filter(|root| !root.trim().is_empty())
}

/// Where the verifier reads `rel`: inside a namespace directory under
/// `.archon/`, never a file directly in it (engine configuration lives
/// there) and never the engine's run store. Compared case-blind, as the
/// filesystem may be.
fn destination(project_root: &str, rel: &str) -> Option<PathBuf> {
    let absolute =
        crate::v2::project_artifact_stamping::project_artifact_destination(project_root, rel)?;
    let absolute = PathBuf::from(absolute);
    let inside: Vec<String> = absolute
        .strip_prefix(project_root)
        .ok()?
        .components()
        .map(|part| part.as_os_str().to_string_lossy().to_ascii_lowercase())
        .collect();
    match inside.as_slice() {
        [archon, namespace, _, ..] if archon == ".archon" && namespace != "workflows" => {
            Some(absolute)
        }
        _ => None,
    }
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

/// One materialization receipt of this run, with the landing it belongs to.
#[derive(Debug, Clone)]
pub(crate) struct RunMaterialization {
    pub(crate) stage_id: String,
    pub(crate) item_id: String,
    pub(crate) receipt: MaterializedDeliverable,
}

/// Every materialization receipt the run's LANDED manifests carry. A failed
/// or unapplied manifest records none, and is skipped if it somehow does.
pub(crate) fn run_materializations(run_root: &Path) -> Vec<RunMaterialization> {
    let stages = run_root.join("write-coordination").join("stages");
    let mut found = Vec::new();
    let Ok(stage_dirs) = std::fs::read_dir(&stages) else {
        return found;
    };
    for stage in stage_dirs.flatten() {
        let Ok(manifests) = std::fs::read_dir(stage.path().join("manifests")) else {
            continue;
        };
        for entry in manifests.flatten() {
            let Some(manifest) = std::fs::read(entry.path())
                .ok()
                .and_then(|bytes| serde_json::from_slice::<PatchManifest>(&bytes).ok())
            else {
                continue;
            };
            let landed = matches!(
                manifest.status,
                ManifestStatus::Applied
                    | ManifestStatus::IdempotentNoop
                    | ManifestStatus::SkippedIgnored
            );
            if !landed {
                continue;
            }
            found.extend(
                manifest
                    .materialized
                    .values()
                    .map(|receipt| RunMaterialization {
                        stage_id: manifest.stage_id.clone(),
                        item_id: manifest.item_id.to_string(),
                        receipt: receipt.clone(),
                    }),
            );
        }
    }
    found
}

#[cfg(test)]
#[path = "materialize_tests.rs"]
mod tests;
