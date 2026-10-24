use serde::{Deserialize, Serialize};

use super::*;

/// The paths the copy builder actually made. Judges derive their site tree
/// from these paths, never from the corresponding live paths.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CopyLayout {
    pub root: PathBuf,
    pub repository: PathBuf,
    pub project: PathBuf,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub siblings: Vec<SiblingRevision>,
    /// Sibling setup named by each check. The top-level list is the shared
    /// materialized layout; reports use this per-check view.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub check_siblings: std::collections::BTreeMap<String, Vec<SiblingRevision>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SiblingRevision {
    pub path: PathBuf,
    pub commit: Option<String>,
    /// Canonical site target captured before the disposable copy is removed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<PathBuf>,
}

/// One check result and the copy whose paths gave that result meaning.
/// Live and scratch observations explicitly carry no copy layout.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CheckEvidence {
    pub result: CheckResult,
    pub copy_layout: Option<CopyLayout>,
}

impl std::ops::Deref for CheckEvidence {
    type Target = CheckResult;

    fn deref(&self) -> &Self::Target {
        &self.result
    }
}

impl CheckEvidence {
    pub fn from_results(results: Vec<CheckResult>, copy_layout: Option<CopyLayout>) -> Vec<Self> {
        results
            .into_iter()
            .map(|result| Self {
                result,
                copy_layout: copy_layout.clone(),
            })
            .collect()
    }
}

impl CopyLayout {
    /// Issue 420's tree as the constructed copy actually placed it.
    pub fn site_tree(&self) -> Option<(usize, Vec<String>)> {
        let depth = self
            .project
            .strip_prefix(&self.root)
            .ok()?
            .components()
            .count();
        let roots = std::fs::read_dir(&self.root)
            .ok()?
            .flatten()
            .filter_map(|entry| {
                let kind = entry.file_type().ok()?;
                (kind.is_dir() || kind.is_file())
                    .then(|| entry.file_name().to_string_lossy().into_owned())
            })
            .collect();
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
        fault: None,
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
