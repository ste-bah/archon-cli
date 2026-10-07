//! Materialize a private content-addressed audit view from captured wave source.
use super::runtime::Snapshot;
use crate::write_coordinator::worktree_isolation::{SealedSource, capture_sealed_source, run_git};
use crate::write_coordinator::{WriteCoordinatorConfig, WritePlan};
use crate::{WorkflowError, WorkflowResult, WorkflowV2ResultStore};
use std::{collections::BTreeMap, path::Path};

fn error(e: impl std::fmt::Display) -> WorkflowError {
    WorkflowError::StageFailed(format!("repository audit snapshot: {e}"))
}
impl Snapshot {
    /// Materialize a private view of `source`. The git work runs with no run
    /// lock held (Issue 291): ownership is checked in a short critical section
    /// before any file is made and again before the view is returned. A view
    /// the session may no longer publish is removed, and so is a run
    /// directory the work recreated after a successor removed it.
    pub fn from_sealed(
        root: &Path,
        source: &SealedSource,
        plan: &WritePlan,
        store: &WorkflowV2ResultStore,
    ) -> WorkflowResult<Self> {
        store.with_session_write_lock(|| Ok(()))?;
        let namespace = Namespace::observe(store);
        let mut plan = plan.clone();
        plan.isolated_root = store
            .root()
            .join("repository-audit/snapshots")
            .join(uuid::Uuid::new_v4().to_string());
        plan.item_id = "repository-audit".into();
        let built = Self::materialize(root, source, &plan)
            .and_then(|snapshot| store.with_session_write_lock(|| Ok(snapshot)));
        if built.is_err() {
            discard(root, &plan.isolated_root);
            namespace.remove_if_recreated();
        }
        built
    }
    fn materialize(root: &Path, source: &SealedSource, plan: &WritePlan) -> WorkflowResult<Self> {
        let view = source.assessment_workspace(root, plan).map_err(error)?;
        let identity = String::from_utf8_lossy(
            &run_git(&["rev-parse", "HEAD^{tree}"], &view.plan.isolated_root)
                .map_err(error)?
                .stdout,
        )
        .trim()
        .to_string();
        let listing = run_git(&["ls-files", "-z"], &view.plan.isolated_root)
            .map_err(error)?
            .stdout;
        let paths = listing
            .split(|b| *b == 0)
            .filter(|p| !p.is_empty())
            .map(|p| String::from_utf8(p.to_vec()).map_err(error))
            .collect::<WorkflowResult<Vec<_>>>()?;
        Ok(Self {
            identity,
            root: view.plan.isolated_root,
            paths,
        })
    }
    /// Capture `paths` of `root` and materialize the view. Reading the source
    /// takes no run lock; [`Self::from_sealed`] fences the view.
    pub fn capture(
        root: &Path,
        paths: &[String],
        store: &WorkflowV2ResultStore,
    ) -> WorkflowResult<Self> {
        let plan = snapshot_plan(root, paths, store)?;
        let source = capture_sealed_source(root, &plan, &WriteCoordinatorConfig::default())
            .map_err(error)?;
        Self::from_sealed(root, &source, &plan, store)
    }
    pub fn content_index(&self) -> WorkflowResult<BTreeMap<String, String>> {
        let listing = run_git(&["ls-tree", "-r", "-z", "HEAD"], &self.root)
            .map_err(error)?
            .stdout;
        let mut index = BTreeMap::new();
        for row in listing.split(|b| *b == 0).filter(|r| !r.is_empty()) {
            let row = std::str::from_utf8(row).map_err(error)?;
            let (meta, path) = row
                .split_once('\t')
                .ok_or_else(|| error("invalid tree entry"))?;
            index.insert(path.into(), meta.into());
        }
        Ok(index)
    }
}
pub fn snapshot_plan(
    root: &Path,
    paths: &[String],
    store: &WorkflowV2ResultStore,
) -> WorkflowResult<WritePlan> {
    use crate::write_coordinator::write_plan::{TargetFilesSource, normalize_target};
    Ok(WritePlan {
        run_id: "repository-audit".into(),
        stage_id: "repository-audit".into(),
        item_id: "repository-audit".into(),
        canonical_root: root.into(),
        isolated_root: store.root().join("repository-audit/capture"),
        target_files: paths
            .iter()
            .map(|p| normalize_target(p, root).map_err(error))
            .collect::<WorkflowResult<Vec<_>>>()?,
        target_dir_scopes: vec![],
        target_files_source: TargetFilesSource::Item,
        read_context_files: vec![],
        verify_inputs: vec![],
        baseline_id: "sealed".into(),
        workspace_boundary_required: true,
        resource_keys: Default::default(),
    })
}

/// Remove a view that was not published. Best effort, reported: the view is
/// private and unreferenced, so a leftover is waste, never evidence.
pub(super) fn discard(root: &Path, view: &Path) {
    if !view.exists() {
        return;
    }
    let path = view.to_string_lossy().into_owned();
    if let Err(error) = run_git(&["worktree", "remove", "--force", &path], root) {
        tracing::warn!(view = %view.display(), %error, "unpublished audit view not removed by git");
    }
    if view.exists()
        && let Err(error) = std::fs::remove_dir_all(view)
    {
        tracing::warn!(view = %view.display(), %error, "unpublished audit view not removed");
    }
    let _ = run_git(&["worktree", "prune"], root);
}

/// The run directory a view is made under, as it was before the work.
struct Namespace {
    dir: Option<std::path::PathBuf>,
    #[cfg(unix)]
    identity: Option<(u64, u64)>,
}
impl Namespace {
    fn observe(store: &WorkflowV2ResultStore) -> Self {
        let dir = store.root().parent().map(Path::to_path_buf);
        Self {
            #[cfg(unix)]
            identity: dir.as_deref().and_then(identity),
            dir,
        }
    }
    /// A successor removed the run while the view was made, and making the
    /// view recreated its directory: remove that recreation again.
    fn remove_if_recreated(&self) {
        let Some(dir) = &self.dir else {
            return;
        };
        #[cfg(unix)]
        let recreated = identity(dir).is_some_and(|now| Some(now) != self.identity);
        #[cfg(not(unix))]
        let recreated = dir.exists() && !dir.join("state.json").exists();
        if recreated
            && !dir.join("state.json").exists()
            && let Err(error) = std::fs::remove_dir_all(dir)
        {
            tracing::warn!(dir = %dir.display(), %error, "recreated run directory not removed");
        }
    }
}
#[cfg(unix)]
fn identity(dir: &Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(dir)
        .ok()
        .map(|meta| (meta.dev(), meta.ino()))
}
