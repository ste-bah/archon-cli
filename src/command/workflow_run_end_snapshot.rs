//! Launch-time run-end observer eligibility snapshot collection.
//!
//! This probe is deliberately non-blocking: it opts a normal authored task run
//! in when any frozen-chain artifact is present, but it neither validates the
//! chain nor rejects launch. Validation belongs to the post-terminal observer.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use archon_workflow::task_set_contract::{
    ACCEPTANCE_CONTRACT_FILE, ACCEPTANCE_LOCK_FILE, AcceptancePin, TASK_SKELETON_FILE,
    TASK_SKELETON_LOCK_FILE,
};
use archon_workflow::task_universe::WorkflowV2TaskUniverse;
use archon_workflow::{
    PortableAcceptanceIdentityV1, RUN_END_OBSERVER_EXPECTED_ARTIFACT_PATHS,
    RUN_END_OBSERVER_SNAPSHOT_SCHEMA_VERSION, RunEndAcceptanceObserverSnapshotV1, WorkflowStore,
};

#[path = "workflow_run_start_toolchain.rs"]
pub(super) mod toolchain;

pub(super) fn collect_run_end_observer_snapshot(
    store: &WorkflowStore,
    universe: &WorkflowV2TaskUniverse,
) -> Option<RunEndAcceptanceObserverSnapshotV1> {
    let project_root = project_root(store)?;
    let task_root = canonical_task_root(project_root, universe)?;
    let pin_path = crate::command::workflow_task_set::acceptance_pin_path(project_root, &task_root);
    let task_artifacts = [
        ACCEPTANCE_CONTRACT_FILE,
        ACCEPTANCE_LOCK_FILE,
        TASK_SKELETON_FILE,
        TASK_SKELETON_LOCK_FILE,
    ];
    if !task_artifacts
        .iter()
        .any(|name| task_root.join(name).exists())
        && !pin_path.exists()
    {
        return None;
    }
    // Issue 294: the pin and the native binding are one version. A set that
    // cannot be locked is read as before, and the reason is logged.
    let _read = crate::command::workflow_task_set::ChainRead::begin(&pin_path, &task_root)
        .inspect_err(|error| {
            tracing::warn!(error = %format!("{error:#}"), "launch snapshot read without the publish lock");
        })
        .ok();
    let portable_acceptance_identity = std::fs::read(&pin_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<AcceptancePin>(&bytes).ok())
        .map(|pin| PortableAcceptanceIdentityV1 {
            freeze_event_id: pin.freeze_event_id,
            acceptance_digest: pin.acceptance_digest,
            skeleton_digest: pin.skeleton_digest,
        });
    Some(RunEndAcceptanceObserverSnapshotV1 {
        native_execution: match crate::command::acceptance_scratch_policy::capture(
            project_root,
            &task_root,
        ) {
            Ok(Some(binding)) => {
                Some(serde_json::to_value(binding).expect("native binding serializes"))
            }
            Ok(None) => None,
            Err(error) => Some(serde_json::json!({"capture_error":error.to_string()})),
        },
        schema_version: RUN_END_OBSERVER_SNAPSHOT_SCHEMA_VERSION,
        canonical_task_root_identity: task_root.display().to_string(),
        expected_artifact_paths: RUN_END_OBSERVER_EXPECTED_ARTIFACT_PATHS
            .into_iter()
            .map(str::to_string)
            .collect(),
        portable_acceptance_identity,
        // This binary records a lineage link for every sanctioned republish,
        // so any change after this launch that its lineage does not record
        // is refused.
        lineage_recording: Some(archon_workflow::task_set_lineage::LINEAGE_RECORDING_V1),
    })
}

pub(super) fn project_root(store: &WorkflowStore) -> Option<&Path> {
    let archon = store.root().parent()?;
    (archon.file_name()?.to_str()? == ".archon")
        .then(|| archon.parent())
        .flatten()
}

pub(super) fn canonical_task_root(
    project_root: &Path,
    universe: &WorkflowV2TaskUniverse,
) -> Option<PathBuf> {
    let mut roots = BTreeSet::new();
    for task in &universe.tasks {
        let source = Path::new(&task.source_path);
        let source = if source.is_absolute() {
            source.to_path_buf()
        } else {
            project_root.join(source)
        };
        roots.insert(
            source
                .parent()?
                .canonicalize()
                .map(archon_shell::paths::plain)
                .ok()?,
        );
    }
    match roots.into_iter().collect::<Vec<_>>().as_slice() {
        [root] => Some(root.clone()),
        _ => None,
    }
}

/// The one launch-time refusal about the bound chain: a contract carrying a
/// check the judge did not accept can never pass, so no run is started on it.
/// Everything else about the chain stays the observer's to validate.
pub(super) fn refuse_unaccepted_launch(
    store: &WorkflowStore,
    universe: Option<&WorkflowV2TaskUniverse>,
) -> anyhow::Result<()> {
    let Some((project, universe)) = project_root(store).zip(universe) else {
        return Ok(());
    };
    let Some(task_root) = canonical_task_root(project, universe) else {
        return Ok(());
    };
    crate::command::workflow_task_set::republish::refuse_unaccepted_launch(project, &task_root)
}

/// Resume settles any publish of the bound task set a crash interrupted
/// (Issue 271) before the run reads the set again.
pub(crate) fn recover_bound_task_set(
    store: &WorkflowStore,
    universe: Option<&WorkflowV2TaskUniverse>,
) -> anyhow::Result<()> {
    let Some((project, universe)) = project_root(store).zip(universe) else {
        return Ok(());
    };
    let Some(task_root) = canonical_task_root(project, universe) else {
        return Ok(());
    };
    let pin_path = crate::command::workflow_task_set::acceptance_pin_path(project, &task_root);
    crate::command::workflow_task_set::recover_interrupted_publish(&pin_path, &task_root)?;
    Ok(())
}
