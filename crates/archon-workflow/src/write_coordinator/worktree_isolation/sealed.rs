//! A pinned source commit and captured overlay, independent of later live edits.
use super::*;

#[derive(Clone, Debug)]
pub struct SealedSource {
    pub base_commit: String,
    pub baseline: CanonicalBaseline,
}

pub fn capture_sealed_source(
    root: &Path, plan: &WritePlan, cfg: &WriteCoordinatorConfig,
) -> Result<SealedSource, IsolationError> {
    let base_commit = String::from_utf8_lossy(&run_git(&["rev-parse", "HEAD"], root)?.stdout)
        .trim().to_string();
    let baseline = capture_canonical_baseline_at(root, plan, &plan.verify_inputs, cfg, &base_commit)?;
    Ok(SealedSource { base_commit, baseline })
}

pub fn create_item_workspace_from_sealed(
    root: &Path, plan: &WritePlan, source: &SealedSource,
) -> Result<ItemWorkspace, IsolationError> {
    create_item_workspace_at(root, plan, &source.baseline, &source.base_commit, true)
}

impl SealedSource {
    /// Private assessment view: no symlinked live dependency/cache directories.
    pub fn assessment_workspace(&self, root: &Path, plan: &WritePlan) -> Result<ItemWorkspace, IsolationError> {
        create_item_workspace_at(root, plan, &self.baseline, &self.base_commit, false)
    }

    /// Preserve each assignment's apply checks without expanding its write plan.
    pub fn baseline_for(&self, plan: &WritePlan) -> CanonicalBaseline {
        let mut baseline = self.baseline.clone();
        baseline.declared_target_meta.retain(|path, _| plan.target_files.iter().any(|p| p.as_str() == *path));
        baseline.verify_input_meta.retain(|path, _| plan.verify_inputs.iter().any(|p| p.as_str() == *path));
        baseline
    }
}
