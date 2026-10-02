//! The project root's copy of a TRACKED input kept in step with a landing
//! that changed it (split from `project_inputs_apply`; see its module doc).

use std::path::Path;

use crate::write_coordinator::PatchManifest;
use crate::write_coordinator::project_inputs::{
    CaptureRecord, ProjectInputPolicy, capture_path, file_state, read_json, read_no_follow,
    write_file,
};
use crate::write_coordinator::worktree_isolation::run_git;

use super::super::project_inputs_ledger::append;
use super::{Decider, keep};

/// Keep the project root's copy of every tracked input `manifest` landed in
/// step with the repository. `Some(reason)` for any copy left as it was.
///
/// Batch K (I1): a tracked input the landing made repository test material
/// is never copied to the project root -- whether or not the project had a
/// copy, since acceptance otherwise reads the repository's -- and each
/// finding is appended to `fixtures`.
pub(super) fn sync_tracked(
    run_root: &Path,
    canonical_root: &Path,
    manifest: &PatchManifest,
    fixtures: &mut Vec<String>,
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
    let index = crate::write_coordinator::fixture_provenance::FixtureIndex::load(
        canonical_root,
        &policy.inputs,
    );
    for rel in paths {
        let project_copy = policy.project.join(rel);
        let now = file_state(&project_copy);
        let post = file_state(&canonical_root.join(rel));
        if post != "absent"
            && let Some(reason) = crate::write_coordinator::fixture_provenance::refuse_test_material(
                run_root,
                &index,
                (&manifest.stage_id, manifest.item_id.as_str()),
                &[(rel.clone(), canonical_root.join(rel))],
                fixtures,
            )
        {
            let reason = format!(
                "{rel}: this tracked input was not brought in step with the landing: {reason}"
            );
            lines.push(decider.line(rel, "sync_refused", &now, &post, &reason));
            refusals.push(reason);
            continue;
        }
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
