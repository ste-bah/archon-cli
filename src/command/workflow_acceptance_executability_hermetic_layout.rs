use super::*;

/// The paths the copy builder actually made. Judges derive their site tree
/// from these paths, never from the corresponding live paths.
#[derive(Clone, Debug)]
pub struct CopyLayout {
    pub root: PathBuf,
    pub repository: PathBuf,
    pub project: PathBuf,
}

impl CopyLayout {
    /// Issue 420's tree as the constructed copy actually placed it.
    pub fn site_tree(&self) -> Option<(usize, &'static [&'static str])> {
        const HERMETIC: &[&str] = &["repo", "project"];
        let depth = self
            .project
            .strip_prefix(&self.root)
            .ok()?
            .components()
            .count();
        let roots = if self.project.starts_with(&self.repository) {
            &HERMETIC[..1]
        } else {
            HERMETIC
        };
        Some((depth, roots))
    }
}

pub struct CopyRun {
    pub results: Vec<CheckResult>,
    pub layout: CopyLayout,
}

/// The warm build directory for `repository`'s copies.
pub fn warm_target(parent: &Path, repository: &Path) -> PathBuf {
    let key = content_digest(
        (archon_shell::paths::canonicalize(repository))
            .unwrap_or_else(|_| repository.to_path_buf())
            .to_string_lossy()
            .as_bytes(),
    );
    parent.join(WARM_TARGETS).join(&key[..16])
}

pub fn deferred(id: &str) -> CheckResult {
    CheckResult {
        classification: None,
        acceptance_id: id.to_string(),
        exit_code: None,
        quota_walk_count: 0,
        stdout: Vec::new(),
        stderr: Vec::new(),
        operational_error: Some(CHECK_DEFERRED.to_string()),
        environment_note: None,
    }
}
