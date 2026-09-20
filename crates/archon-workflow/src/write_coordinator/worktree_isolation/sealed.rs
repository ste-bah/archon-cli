//! A pinned source commit and captured overlay, independent of later live edits.
use std::collections::BTreeSet;

use super::*;

/// What the materialized file does not share with the captured one. Git retains
/// executable bits, not arbitrary local permission bits.
fn differences(actual: &FileMeta, expected: &FileMeta) -> Vec<&'static str> {
    if actual.exists != expected.exists {
        return vec![if expected.exists {
            "exists (captured, missing in view)"
        } else {
            "exists (absent at capture, present in view)"
        }];
    }
    let mut differed = Vec::new();
    if actual.blake3_hex != expected.blake3_hex {
        differed.push("content hash");
    }
    if actual.symlink_target != expected.symlink_target {
        differed.push("symlink target");
    }
    if (actual.mode & 0o111 != 0) != (expected.mode & 0o111 != 0) {
        differed.push("executable bit");
    }
    differed
}

#[derive(Clone, Debug)]
pub struct SealedSource {
    pub base_commit: String,
    pub baseline: CanonicalBaseline,
    /// Declared targets and verify inputs the CANONICAL repository ignores and
    /// does not track. Decided once, at capture time, by the repository that
    /// owns the ignore rules: capture never reads such a file and `git add -A`
    /// never seals one, so no materialized view can hold it (Issue-50). A
    /// materialized root is not asked — it need not be a git checkout.
    pub unsealable: BTreeSet<String>,
}

pub fn capture_sealed_source(
    root: &Path,
    plan: &WritePlan,
    cfg: &WriteCoordinatorConfig,
) -> Result<SealedSource, IsolationError> {
    let base_commit = String::from_utf8_lossy(&run_git(&["rev-parse", "HEAD"], root)?.stdout)
        .trim()
        .to_string();
    let mut baseline =
        capture_canonical_baseline_at(root, plan, &plan.verify_inputs, cfg, &base_commit)?;
    baseline.untracked_files = support_files::capture_all_untracked(root, cfg.max_file_bytes)?;
    // Context metadata controls reproduction only; ownership remains in plan.
    for path in baseline.untracked_files.keys() {
        baseline
            .declared_target_meta
            .insert(path.clone(), file_meta(&root.join(path))?);
    }
    let obligated: Vec<String> = plan
        .target_files
        .iter()
        .chain(&plan.verify_inputs)
        .map(NormalizedPath::as_str)
        .collect();
    let unsealable = check_ignore(root, &obligated)?.into_iter().collect();
    Ok(SealedSource {
        base_commit,
        baseline,
        unsealable,
    })
}

pub fn create_item_workspace_from_sealed(
    root: &Path,
    plan: &WritePlan,
    source: &SealedSource,
) -> Result<ItemWorkspace, IsolationError> {
    let workspace =
        create_item_workspace_at(root, plan, &source.baseline, &source.base_commit, true)?;
    source.validate_materialized(&workspace.plan.isolated_root, plan)?;
    Ok(workspace)
}

impl SealedSource {
    /// Private assessment view: no symlinked live dependency/cache directories.
    pub fn assessment_workspace(
        &self,
        root: &Path,
        plan: &WritePlan,
    ) -> Result<ItemWorkspace, IsolationError> {
        let workspace =
            create_item_workspace_at(root, plan, &self.baseline, &self.base_commit, false)?;
        self.validate_materialized(&workspace.plan.isolated_root, plan)?;
        Ok(workspace)
    }

    /// Every captured file must be reproduced byte-for-byte in `root`, except
    /// the ones the canonical repository ignores: those were never captured and
    /// the write layer already reports such a deliverable as `skipped_ignored`,
    /// so their absence is the sealed view's normal shape, obligated or not.
    pub(super) fn validate_materialized(
        &self,
        root: &Path,
        plan: &WritePlan,
    ) -> Result<(), IsolationError> {
        for (path, expected) in self
            .baseline
            .declared_target_meta
            .iter()
            .chain(&self.baseline.verify_input_meta)
        {
            let obligated = plan
                .target_files
                .iter()
                .chain(&plan.verify_inputs)
                .any(|target| target.as_str() == *path);
            if self.unsealable.contains(path) {
                eprintln!(
                    "sealed source: ignored untracked file '{path}' omitted from the materialized view (obligated by plan: {obligated})"
                );
                continue;
            }
            let differed = differences(&file_meta(&root.join(path))?, expected);
            if !differed.is_empty() {
                return Err(IsolationError::SealedMismatch {
                    path: path.clone(),
                    obligated,
                    differed: differed.join(", "),
                });
            }
        }
        Ok(())
    }

    /// Preserve each assignment's apply checks without expanding its write plan.
    pub fn baseline_for(&self, plan: &WritePlan) -> CanonicalBaseline {
        let mut baseline = self.baseline.clone();
        baseline
            .declared_target_meta
            .retain(|path, _| plan.target_files.iter().any(|p| p.as_str() == *path));
        baseline
            .verify_input_meta
            .retain(|path, _| plan.verify_inputs.iter().any(|p| p.as_str() == *path));
        baseline
    }
}
