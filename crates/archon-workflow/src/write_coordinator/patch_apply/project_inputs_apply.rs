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
use crate::write_coordinator::project_inputs::{
    CaptureRecord, ProjectInputPolicy, capture_path, captured_bytes_dir, file_state, meta_state,
    read_json, read_no_follow, refuse_links, write_file,
};
use crate::write_coordinator::worktree_isolation::run_git;

use super::project_inputs_ledger::{ProjectInputLanding, append, run_project_input_landings};

/// Where the project's copy of `rel` is kept before a landing of
/// (`stage`, `item`) replaces or removes it: never overwritten once kept.
fn kept_path(run_root: &Path, manifest: &PatchManifest, rel: &str) -> PathBuf {
    run_root
        .join("write-coordination")
        .join("project-inputs-replaced")
        .join(&manifest.stage_id)
        .join(manifest.item_id.as_str())
        .join(rel)
}

/// Keep the project's current copy of `rel` before it is replaced.
fn keep(
    run_root: &Path,
    manifest: &PatchManifest,
    rel: &str,
    current: &Path,
) -> Result<PathBuf, String> {
    let kept = kept_path(run_root, manifest, rel);
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

/// What a landing replaced, newest last, so a failure can put it back.
#[derive(Default)]
struct Undo(Vec<(PathBuf, Option<Vec<u8>>)>);

impl Undo {
    fn restore(self, root: &Path) -> Result<(), String> {
        let mut failed = Vec::new();
        for (path, before) in self.0.into_iter().rev() {
            let restored = match before {
                Some(bytes) => write_file(root, &path, &bytes).map(|_| ()),
                None => std::fs::remove_file(&path),
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

/// Apply `manifest`'s captured project-input changes to the project root.
/// `Some(reason)` when they were refused (none applied), else `None`.
pub(super) fn apply(run_root: &Path, manifest: &PatchManifest) -> Option<String> {
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
    let Some(policy) = ProjectInputPolicy::for_run(run_root) else {
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
    let decided = |rel: &str, change: &crate::write_coordinator::project_inputs::InputChange| {
        ledger.iter().any(|line| {
            line.outcome == "applied"
                && line.stage_id == stage
                && line.item_id == item
                && line.path == rel
                && line.before == change.baseline
                && line.after == change.post
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
    let bytes_dir = captured_bytes_dir(run_root, stage, item);
    let mut undo = Undo::default();
    let mut lines = Vec::new();
    let mut total = 0u64;
    let outcome = (|| -> Result<(), String> {
        for (rel, change) in &pending {
            let destination = policy
                .destination(rel)
                .map_err(|why| format!("{rel}: {why}"))?;
            refuse_links(&policy.project, &destination).map_err(|e| format!("{rel}: {e}"))?;
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
                std::fs::remove_file(&destination).map_err(|e| format!("{rel}: {e}"))?;
                undo.0.push((destination, Some(before)));
            } else {
                let bytes = read_no_follow(&bytes_dir.join(rel))
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
                let before = write_file(&policy.project, &destination, &bytes)
                    .map_err(|e| format!("{rel}: {e}"))?;
                undo.0.push((destination, before));
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
    let reason = match undo.restore(&policy.project) {
        Ok(()) => reason,
        Err(error) => {
            format!("{reason}; {error}: the project root is left changed, a person must look")
        }
    };
    let refused: Vec<_> = pending
        .iter()
        .map(|(rel, change)| {
            let now = file_state(&policy.project.join(rel.as_str()));
            decider.line(rel, "refused", &now, &change.post, &reason)
        })
        .collect();
    Some(match append(run_root, &refused) {
        Ok(()) => reason,
        Err(error) => format!("{reason}; the refusal could not be logged: {error}"),
    })
}

/// Keep the project root's copy of every tracked input `manifest` landed in
/// step with the repository. `Some(reason)` for any copy left as it was.
pub(super) fn sync_tracked(
    run_root: &Path,
    canonical_root: &Path,
    manifest: &PatchManifest,
) -> Option<String> {
    // Only acceptance that overlays the inputs on the repository can see a
    // tracked input collide with the project's copy.
    let policy = ProjectInputPolicy::for_run(run_root).filter(|policy| policy.combined)?;
    let mut paths: Vec<&String> = manifest
        .changed_files
        .iter()
        .chain(&manifest.created_files)
        .chain(&manifest.deleted_files)
        .filter(|path| policy.covers(path))
        .collect();
    paths.sort();
    paths.dedup();
    let capture: Option<CaptureRecord> = read_json(&capture_path(
        run_root,
        &manifest.stage_id,
        manifest.item_id.as_str(),
    ));
    let decider = Decider {
        manifest,
        task_ids: capture.map(|c| c.task_ids).unwrap_or_default(),
    };
    let mut lines = Vec::new();
    let mut refusals = Vec::new();
    for rel in paths {
        let project_copy = policy.project.join(rel);
        let now = file_state(&project_copy);
        let post = file_state(&canonical_root.join(rel));
        // No copy: the scratch reads the repository's; one in step: done. A
        // copy the repository no longer has collides with nothing and is
        // the project's data: it stays.
        if now == "absent" || now == post || post == "absent" {
            continue;
        }
        let spec = format!("{}:{rel}", manifest.baseline_commit);
        let pre = run_git(&["show", &spec], canonical_root)
            .map(|output| blake3::hash(&output.stdout).to_hex().to_string())
            .unwrap_or_else(|_| "absent".into());
        // A copy that followed neither side is kept, never lost, and then
        // replaced: the repository's landed copy is the one acceptance
        // must see, and a collision would fail every later round.
        let mut note = String::new();
        let synced = policy.destination(rel).and_then(|destination| {
            let kept = keep(run_root, manifest, rel, &destination)?;
            if now != pre {
                note = format!(
                    "the project's copy followed neither the repository's copy before nor after this landing; it was kept at {} and replaced",
                    kept.display()
                );
            }
            read_no_follow(&canonical_root.join(rel))
                .and_then(|bytes| write_file(&policy.project, &destination, &bytes))
                .map(|_| ())
                .map_err(|e| e.to_string())
        });
        match synced {
            Ok(()) => lines.push(decider.line(rel, "synced", &now, &post, &note)),
            Err(why) => {
                let reason = format!(
                    "{rel}: the project root's copy of this tracked input was not brought in step with the landing: {why}; acceptance scratch refuses a nonidentical copy"
                );
                lines.push(decider.line(rel, "sync_refused", &now, &post, &reason));
                refusals.push(reason);
            }
        }
    }
    if let Err(error) = append(run_root, &lines) {
        refusals.push(format!(
            "the project input log could not be written: {error}"
        ));
    }
    (!refusals.is_empty()).then(|| refusals.join("; "))
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
    let mut refusals: Vec<String> = apply(run_root, manifest).into_iter().collect();
    if patch_applied {
        refusals.extend(sync_tracked(run_root, canonical_root, manifest));
    }
    if !refusals.is_empty() {
        rec.project_input_refusals
            .push((manifest.item_id.clone(), refusals.join("; ")));
    }
}

#[cfg(test)]
#[path = "project_inputs_apply_tests.rs"]
mod tests;
