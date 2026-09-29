//! Batch K (I1): the applied tree is judged before a landing is recorded.
//!
//! Two landing paths put a patch's bytes where acceptance reads them as
//! project data without passing through the project-input capture: a
//! TRACKED project input the patch changed or created (the repository's
//! copy is what acceptance scratch reads, or what `sync_tracked` copies to
//! the project root), and a declared deliverable `materialize` placed. Both
//! are judged here, after `git apply` and before the landing is recorded, so
//! the index sees what the working tree now holds -- HEAD plus every test
//! file this wave has written but not committed, this patch's own included.
//! A hit fails the landing: the caller puts the tracked files and the
//! placed copies back, so nothing of it is committed or left in the project.

use std::path::PathBuf;

use super::PatchManifest;
use super::materialize::Failure;
use crate::write_coordinator::fixture_provenance::{FixtureIndex, refuse_test_material};
use crate::write_coordinator::project_inputs::ProjectInputPolicy;

pub(super) fn judge_applied(
    run_root: &std::path::Path,
    canonical_root: &std::path::Path,
    manifest: &PatchManifest,
) -> Option<Failure> {
    let policy = ProjectInputPolicy::for_run(run_root);
    let inputs = policy
        .as_ref()
        .map(|p| p.inputs.clone())
        .unwrap_or_default();
    let mut files: Vec<(String, PathBuf)> = (manifest.changed_files.iter())
        .chain(&manifest.created_files)
        .filter(|rel| policy.as_ref().is_some_and(|policy| policy.covers(rel)))
        .map(|rel| (rel.clone(), canonical_root.join(rel)))
        .collect();
    files.extend(
        (manifest.materialized.iter())
            .map(|(rel, receipt)| (rel.clone(), PathBuf::from(&receipt.destination))),
    );
    if files.is_empty() {
        return None;
    }
    let index = FixtureIndex::load(canonical_root, &inputs);
    let mut fixtures = Vec::new();
    let ids = (manifest.stage_id.as_str(), manifest.item_id.as_str());
    let reason = refuse_test_material(run_root, &index, ids, &files, &mut fixtures)?;
    Some(Failure {
        reason,
        attention: None,
        fixtures,
    })
}
