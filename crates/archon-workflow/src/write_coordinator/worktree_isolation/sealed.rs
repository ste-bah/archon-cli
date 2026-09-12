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
    let mut baseline = capture_canonical_baseline_at(root, plan, &plan.verify_inputs, cfg, &base_commit)?;
    baseline.untracked_files = support_files::capture_all_untracked(root, cfg.max_file_bytes)?;
    // Context metadata controls reproduction only; ownership remains in plan.
    for path in baseline.untracked_files.keys() {
        baseline.declared_target_meta.insert(path.clone(), file_meta(&root.join(path))?);
    }
    Ok(SealedSource { base_commit, baseline })
}

pub fn create_item_workspace_from_sealed(
    root: &Path, plan: &WritePlan, source: &SealedSource,
) -> Result<ItemWorkspace, IsolationError> {
    let workspace = create_item_workspace_at(root, plan, &source.baseline, &source.base_commit, true)?;
    source.validate_materialized(&workspace.plan.isolated_root, plan)?;
    Ok(workspace)
}

impl SealedSource {
    /// Private assessment view: no symlinked live dependency/cache directories.
    pub fn assessment_workspace(&self, root: &Path, plan: &WritePlan) -> Result<ItemWorkspace, IsolationError> {
        let workspace = create_item_workspace_at(root, plan, &self.baseline, &self.base_commit, false)?;
        self.validate_materialized(&workspace.plan.isolated_root, plan)?;
        Ok(workspace)
    }

    fn validate_materialized(&self, root: &Path, plan: &WritePlan) -> Result<(), IsolationError> {
        for (path, expected) in self.baseline.declared_target_meta.iter()
            .chain(&self.baseline.verify_input_meta) {
            let obligated = plan.target_files.iter().chain(&plan.verify_inputs)
                .any(|target| target.as_str() == *path);
            if !obligated && run_git(&["check-ignore", "-q", "--", path], root).is_ok()
                && !run_git(&["ls-files", "--error-unmatch", "--", path], root).is_ok() {
                eprintln!("repository audit snapshot: ignored undeclared file '{path}' omitted");
                continue;
            }
            let actual = file_meta(&root.join(path))?;
            // Git retains executable bits, not arbitrary local permission bits.
            if actual.exists != expected.exists || actual.blake3_hex != expected.blake3_hex
                || actual.symlink_target != expected.symlink_target
                || (actual.mode & 0o111 != 0) != (expected.mode & 0o111 != 0) {
                return Err(IsolationError::HashMismatch { path: path.clone() });
            }
        }
        Ok(())
    }

    /// Preserve each assignment's apply checks without expanding its write plan.
    pub fn baseline_for(&self, plan: &WritePlan) -> CanonicalBaseline {
        let mut baseline = self.baseline.clone();
        baseline.declared_target_meta.retain(|path, _| plan.target_files.iter().any(|p| p.as_str() == *path));
        baseline.verify_input_meta.retain(|path, _| plan.verify_inputs.iter().any(|p| p.as_str() == *path));
        baseline
    }
}
