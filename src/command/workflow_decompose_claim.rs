//! Claiming a task root for one fixed decomposition run at a time.

use super::*;

pub(crate) fn create_claimed_run(
    store: &WorkflowStore,
    task_root: &Path,
    spec: WorkflowSpec,
    state: &FixedDecompositionStateV1,
) -> Result<archon_workflow::WorkflowRun> {
    store.with_store_lock(|locked| {
        refuse_active_task_root(locked, task_root)
            .map_err(|error| archon_workflow::WorkflowError::PolicyDenied(error.to_string()))?;
        let run = locked.create_run(spec)?;
        if let Err(error) = locked.write_run_json(&run.id, FIXED_DECOMPOSITION_STATE_PATH, state) {
            let run_dir = locked.run_dir(&run.id);
            if let Err(cleanup) = std::fs::remove_dir_all(&run_dir) {
                return Err(archon_workflow::WorkflowError::StateCorrupt(format!(
                    "fixed task-root claim failed ({error}); incomplete run {} could not be removed ({cleanup})",
                    run.id
                )));
            }
            return Err(error);
        }
        Ok(run)
    })
    .map_err(Into::into)
}

fn refuse_active_task_root(store: &WorkflowStore, task_root: &Path) -> Result<()> {
    let identity = path_text(task_root);
    for run in store.list_runs()? {
        if crate::command::workflow_task_root_reclaim::is_reclaimed(store, &run.id)? {
            continue;
        }
        if matches!(
            run.status,
            archon_workflow::RunStatus::Completed | archon_workflow::RunStatus::Failed
        ) {
            continue;
        }
        let Ok(state) = read_fixed_state(store, &run.id) else {
            continue;
        };
        if state.run_kind == WorkflowRunKind::FixedDecompositionV1
            && path_text(Path::new(&state.identity.task_root_identity)) == identity
        {
            return Err(anyhow!(
                "active fixed decomposition {} already owns task root {}; resume or complete that run, or use workflow reclaim-task-root <RUN_ID> --yes after its executor stops; cancelled fixed runs remain resumable until reclaimed",
                run.id,
                task_root.display()
            ));
        }
    }
    Ok(())
}

pub(super) fn read_fixed_state(
    store: &WorkflowStore,
    run_id: &str,
) -> Result<FixedDecompositionStateV1> {
    read_run_json(store, run_id, FIXED_DECOMPOSITION_STATE_PATH)
}

pub(super) fn read_run_json<T: serde::de::DeserializeOwned>(
    store: &WorkflowStore,
    run_id: &str,
    relative: &str,
) -> Result<T> {
    let path = store.run_dir(run_id).join(relative);
    serde_json::from_slice(
        &std::fs::read(&path)
            .with_context(|| format!("reading fixed decomposition record {}", path.display()))?,
    )
    .with_context(|| format!("parsing fixed decomposition record {}", path.display()))
}
