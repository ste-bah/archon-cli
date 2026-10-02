//! A landing applies its branch's changes to the project's acceptance
//! inputs to the project root (Batch E; see
//! `write_coordinator::project_inputs`).
//!
//! Run by `apply_one` under the repository lock, one landing at a time, AFTER
//! the branch's patch (if any) applied: two branches of one wave that both
//! changed a shared input are applied in item order, and the second finds
//! the first's bytes where its own seed baseline was, so it is refused as
//! stale rather than overwriting them. Per landing, all of its changes apply
//! or none do: a refusal, or a failure part way, puts back every copy
//! already made. Every decision -- applied or refused, with the state before
//! and after -- is appended to the run's log
//! (`write-coordination/project-inputs.jsonl`), each preceded by an intent
//! line; a change already applied there (a resume re-applying the same
//! capture) moves nothing again. A
//! refusal is returned to the caller, which reports it on the branch as a
//! HIGH gap for its tasks, and the residual passes read it from the log.
//!
//! The same module keeps the project root's copy of a TRACKED input in step
//! with a landing that changed it: acceptance scratch overlays the project's
//! copy on the repository and refuses a nonidentical collision, so a landed
//! change the project's copy did not follow would turn the next round into
//! operational errors. The project's copy is replaced by the landed one.
//!
//! Every project copy a landing replaces or removes is first kept under the
//! run (`write-coordination/project-inputs-replaced/<stage>/<item>/`), so
//! none is ever lost; the log names what each held before and after.

use std::path::{Path, PathBuf};

use crate::write_coordinator::PatchManifest;
use crate::write_coordinator::project_inputs::external::{missing_dirs, remove_created, stored};
use crate::write_coordinator::project_inputs::{
    CaptureRecord, ProjectInputPolicy, capture_path, captured_bytes_dir, file_state, meta_state,
    read_json, read_no_follow, refuse_links, write_file,
};

use super::project_inputs_ledger::{ProjectInputLanding, append, run_project_input_landings};

/// Where the project's copy of `rel` is kept before a landing of
/// (`stage`, `item`) replaces or removes it: never overwritten once kept.
/// An external file (Issue-226) is kept under `.external/<its path>`.
fn kept_path(run_root: &Path, manifest: &PatchManifest, rel: &str) -> PathBuf {
    run_root
        .join("write-coordination")
        .join("project-inputs-replaced")
        .join(&manifest.stage_id)
        .join(manifest.item_id.as_str())
        .join(stored(rel))
}

/// Keep the project's current copy of `rel` before it is replaced.
fn keep(
    run_root: &Path,
    manifest: &PatchManifest,
    rel: &str,
    current: &Path,
) -> Result<PathBuf, String> {
    let kept = kept_path(run_root, manifest, rel);
    // Batch L: every state a landing replaces is also kept by its content,
    // so a refused landing can be put back even over an earlier one here.
    if let Ok(bytes) = read_no_follow(current) {
        crate::write_coordinator::input_tripwire::keep_object(run_root, &bytes);
    }
    if !kept.exists() {
        let bytes = read_no_follow(current).map_err(|e| format!("{rel}: {e}"))?;
        std::fs::create_dir_all(kept.parent().unwrap_or(run_root))
            .and_then(|()| std::fs::write(&kept, bytes))
            .map_err(|e| format!("{rel}: its current copy could not be kept: {e}"))?;
    }
    Ok(kept)
}

fn now() -> i64 {
    chrono::Utc::now().timestamp_nanos_opt().unwrap_or(i64::MAX)
}

struct Decider<'a> {
    manifest: &'a PatchManifest,
    task_ids: Vec<String>,
}

impl Decider<'_> {
    fn line(
        &self,
        path: &str,
        outcome: &str,
        before: &str,
        after: &str,
        reason: &str,
    ) -> ProjectInputLanding {
        ProjectInputLanding {
            created_dirs: Vec::new(),
            stage_id: self.manifest.stage_id.clone(),
            item_id: self.manifest.item_id.to_string(),
            task_ids: self.task_ids.clone(),
            path: path.to_string(),
            outcome: outcome.to_string(),
            before: before.to_string(),
            after: after.to_string(),
            reason: reason.to_string(),
            at: now(),
        }
    }
}

/// What a landing replaced, newest last, so a failure can put it back:
/// each file, the tree it lies in, what it held, and the directories its
/// write created (Issue-226: an external root missing until the landing).
#[derive(Default)]
struct Undo(Vec<Replaced>);

struct Replaced {
    tree: PathBuf,
    path: PathBuf,
    before: Option<Vec<u8>>,
    created: Vec<PathBuf>,
}

impl Undo {
    fn push(
        &mut self,
        tree: PathBuf,
        path: PathBuf,
        before: Option<Vec<u8>>,
        created: Vec<PathBuf>,
    ) {
        self.0.push(Replaced {
            tree,
            path,
            before,
            created,
        });
    }

    fn restore(self) -> Result<(), String> {
        let mut failed = Vec::new();
        for Replaced {
            tree,
            path,
            before,
            created,
        } in self.0.into_iter().rev()
        {
            let restored = match before {
                Some(bytes) => write_file(&tree, &path, &bytes).map(|_| ()),
                None => crate::write_coordinator::input_tripwire::remove_input(&path),
            }
            .map_err(|error| error.to_string())
            .and_then(|()| remove_created(&created));
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

/// [`apply_judged`] with no repository to judge provenance against.
#[cfg(test)]
pub(super) fn apply(run_root: &Path, manifest: &PatchManifest) -> Option<String> {
    apply_judged(run_root, None, manifest, &mut Vec::new())
}

/// A refused landing leaves nothing of itself: bytes an earlier,
/// interrupted apply of this very change already placed (the project's copy
/// holds `post` though no `applied` line says so) are put back to the
/// baseline -- removed when there was none, else from the copy kept before
/// that apply replaced it.
fn undo_leftovers(
    run_root: &Path,
    manifest: &PatchManifest,
    policy: &ProjectInputPolicy,
    capture: &CaptureRecord,
    pending: &[(
        &String,
        &crate::write_coordinator::project_inputs::InputChange,
    )],
) {
    for (rel, change) in pending {
        let Ok((tree, destination)) = policy.landing(rel, capture.declared.contains(rel.as_str()))
        else {
            continue;
        };
        if change.post == "deleted" || file_state(&destination) != change.post {
            continue;
        }
        let kept = kept_path(run_root, manifest, rel);
        let restored = match change.baseline.as_str() {
            "absent" => crate::write_coordinator::input_tripwire::remove_input(&destination),
            _ => read_no_follow(&kept)
                .and_then(|bytes| write_file(&tree, &destination, &bytes).map(|_| ())),
        };
        if let Err(error) = restored {
            eprintln!(
                "write-coordination: NEEDS ATTENTION {}/{}: {rel} holds refused bytes and could not be put back: {error}",
                manifest.stage_id, manifest.item_id
            );
        }
    }
}

/// Apply `manifest`'s captured project-input changes to the project root.
/// `Some(reason)` when they were refused (none applied), else `None`.
///
/// Batch K (I1): with `canonical_root`, a landing any of whose files is
/// repository test material (`fixture_provenance`) is refused whole, its
/// bytes kept as evidence and each finding appended to `fixtures`.
pub(super) fn apply_judged(
    run_root: &Path,
    canonical_root: Option<&Path>,
    manifest: &PatchManifest,
    fixtures: &mut Vec<String>,
) -> Option<String> {
    let stage = manifest.stage_id.as_str();
    let item = manifest.item_id.as_str();
    let capture: CaptureRecord = read_json(&capture_path(run_root, stage, item))?;
    if capture.changes.is_empty() {
        return None;
    }
    let decider = Decider {
        manifest,
        task_ids: capture.task_ids.clone(),
    };
    let refuse_all = |reason: String| {
        let lines: Vec<_> = capture
            .changes
            .iter()
            .map(|(rel, change)| decider.line(rel, "refused", "unknown", &change.post, &reason))
            .collect();
        let logged = append(run_root, &lines).err();
        Some(match logged {
            Some(error) => format!("{reason}; the refusal could not be logged: {error}"),
            None => reason,
        })
    };
    let Some(policy) = ProjectInputPolicy::for_landing(run_root) else {
        return refuse_all("the run's project input policy cannot be read".into());
    };
    let ledger = match run_project_input_landings(run_root) {
        Ok(ledger) => ledger,
        Err(error) => return refuse_all(format!("the project input log cannot be read: {error}")),
    };
    // This very change -- same landing, path, baseline and bytes -- applied
    // before (a resume applying the same capture): nothing moves again. A
    // new capture judged from another baseline is a new change, and a
    // refusal is judged afresh: the project's copy may be back.
    // Batch L: an application the host later reverted (its unit's verdict
    // refused it) is no longer in place, so the same change is judged afresh.
    let decided = |rel: &str, change: &crate::write_coordinator::project_inputs::InputChange| {
        let own = |line: &ProjectInputLanding| {
            line.stage_id == stage && line.item_id == item && line.path == rel
        };
        ledger.iter().enumerate().any(|(at, line)| {
            line.outcome == "applied"
                && own(line)
                && line.before == change.baseline
                && line.after == change.post
                && !ledger[at..]
                    .iter()
                    .any(|later| own(later) && later.reverted())
        })
    };
    let pending: Vec<_> = capture
        .changes
        .iter()
        .filter(|(rel, change)| !decided(rel, change))
        .collect();
    if pending.is_empty() {
        return None;
    }
    let bytes_dir = captured_bytes_dir(run_root, stage, item);
    // Batch K (I1): judged before anything is written -- a change already
    // applied above is never judged again, so a resume moves nothing.
    if let Some(repo) = canonical_root {
        use crate::write_coordinator::fixture_provenance as provenance;
        let files: Vec<(String, PathBuf)> = pending
            .iter()
            .filter(|(_, change)| change.post != "deleted")
            .map(|(rel, _)| ((*rel).clone(), bytes_dir.join(stored(rel))))
            .collect();
        let index = provenance::FixtureIndex::load(repo, &policy.inputs);
        if let Some(reason) =
            provenance::refuse_test_material(run_root, &index, (stage, item), &files, fixtures)
        {
            undo_leftovers(run_root, manifest, &policy, &capture, &pending);
            return refuse_all(reason);
        }
    }
    // What is about to be written, before any of it is: a crash part way is
    // never a project-root change the log does not name.
    let intents: Vec<_> = pending
        .iter()
        .map(|(rel, change)| decider.line(rel, "intent", &change.baseline, &change.post, ""))
        .collect();
    if let Err(error) = append(run_root, &intents) {
        return refuse_all(format!(
            "the project input log could not be written: {error}"
        ));
    }
    let mut undo = Undo::default();
    let mut lines = Vec::new();
    let mut total = 0u64;
    let outcome = (|| -> Result<(), String> {
        for (rel, change) in &pending {
            // Batch G2: a declared project artifact outside the inputs is
            // placed by the same rules, for that exact file.
            let (tree, destination) = policy
                .landing(rel, capture.declared.contains(rel.as_str()))
                .map_err(|why| format!("{rel}: {why}"))?;
            refuse_links(&tree, &destination).map_err(|e| format!("{rel}: {e}"))?;
            // A file never copied into the branch is judged by size and
            // time, as it was recorded; everything else by its bytes.
            let hashed = file_state(&destination);
            let now = if change.baseline.starts_with("meta:") {
                meta_state(&destination)
            } else {
                hashed.clone()
            };
            if hashed == change.post || (change.post == "deleted" && now == "absent") {
                let already = "already in place";
                lines.push(decider.line(rel, "applied", &change.baseline, &change.post, already));
                continue;
            }
            if now != change.baseline {
                return Err(format!(
                    "stale baseline at {rel}: the project root's copy changed after this branch was seeded (was {}, now {now}) and was NOT overwritten; re-run the data command against the project's current data",
                    change.baseline
                ));
            }
            // What it replaces is kept first: every project copy a landing
            // overwrote or removed can be put back by hand.
            if now != "absent" {
                keep(run_root, manifest, rel, &destination)?;
            }
            if change.post == "deleted" {
                let before = read_no_follow(&destination).map_err(|e| format!("{rel}: {e}"))?;
                crate::write_coordinator::input_tripwire::remove_input(&destination)
                    .map_err(|e| format!("{rel}: {e}"))?;
                undo.push(tree, destination, Some(before), Vec::new());
            } else {
                let bytes = read_no_follow(&bytes_dir.join(stored(rel)))
                    .map_err(|e| format!("{rel}: capture: {e}"))?;
                if blake3::hash(&bytes).to_hex().to_string() != change.post {
                    return Err(format!("{rel}: the kept bytes do not match the capture"));
                }
                total += bytes.len() as u64;
                if total > policy.limit {
                    return Err(format!(
                        "{rel}: over the {} byte project input cap",
                        policy.limit
                    ));
                }
                // Issue-226: a declared root missing until now is created by
                // this write, recorded so undo and a revert remove it.
                let created = missing_dirs(&tree, &destination);
                let before = match write_file(&tree, &destination, &bytes) {
                    Ok(before) => before,
                    Err(error) => {
                        let _ = remove_created(&created);
                        return Err(format!(
                            "{rel}: the landing could not write {} ({:?}): {error}",
                            destination.display(),
                            error.kind()
                        ));
                    }
                };
                let mut line = decider.line(rel, "applied", &now, &change.post, "");
                line.created_dirs = (created.iter())
                    .map(|dir| dir.display().to_string())
                    .collect();
                undo.push(tree, destination, before, created);
                lines.push(line);
                continue;
            }
            lines.push(decider.line(rel, "applied", &now, &change.post, ""));
        }
        Ok(())
    })();
    let reason = match outcome {
        Ok(()) => match append(run_root, &lines) {
            Ok(()) => return None,
            Err(error) => format!("the project input log could not be written: {error}"),
        },
        Err(reason) => reason,
    };
    let reason = match undo.restore() {
        Ok(()) => reason,
        Err(error) => {
            format!("{reason}; {error}: the project root is left changed, a person must look")
        }
    };
    let refused: Vec<_> = pending
        .iter()
        .map(|(rel, change)| {
            // An external key is absolute: the join is that path.
            let now = file_state(&policy.project.join(rel.as_str()));
            decider.line(rel, "refused", &now, &change.post, &reason)
        })
        .collect();
    Some(match append(run_root, &refused) {
        Ok(()) => reason,
        Err(error) => format!("{reason}; the refusal could not be logged: {error}"),
    })
}

/// Everything a decided landing does to the project's inputs: its branch's
/// changes, and -- when its patch applied -- the tracked inputs it changed.
/// A refusal is recorded on `rec` for the caller to report.
pub(super) fn land(
    run_root: &Path,
    canonical_root: &Path,
    manifest: &PatchManifest,
    patch_applied: bool,
    rec: &mut super::ApplyRecord,
) {
    let mut fixtures = Vec::new();
    let mut refusals: Vec<String> =
        apply_judged(run_root, Some(canonical_root), manifest, &mut fixtures)
            .into_iter()
            .collect();
    if patch_applied {
        refusals.extend(sync_tracked(
            run_root,
            canonical_root,
            manifest,
            &mut fixtures,
        ));
    }
    if !refusals.is_empty() {
        rec.project_input_refusals
            .push((manifest.item_id.clone(), refusals.join("; ")));
    }
    rec.fixture_landings.extend(
        fixtures
            .into_iter()
            .map(|finding| (manifest.item_id.clone(), finding)),
    );
}

#[path = "project_inputs_sync.rs"]
mod sync;
use sync::sync_tracked;

#[cfg(test)]
#[path = "project_inputs_apply_tests.rs"]
mod tests;
