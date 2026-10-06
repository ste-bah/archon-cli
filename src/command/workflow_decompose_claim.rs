//! Claiming a task root for one fixed decomposition run at a time.

use super::*;

/// What a run that failed before any work started writes when it gives its
/// task root back. It holds while the run is still the cancelled run that
/// wrote it; any later lifecycle step bumps the generation and makes the run
/// an owner again, and a resume re-claims the root explicitly first.
const RELEASED_PATH: &str = "decomposition/task-root-released.json";

/// The task root a launch names, canonical. A destination that does not exist
/// yet resolves through its parent, which must exist; the directory itself is
/// created only when the launch claims it, so a launch refused before then
/// leaves nothing behind.
pub(super) fn resolve_task_root(project_root: &Path, tasks: &Path) -> Result<PathBuf> {
    let candidate = if tasks.is_absolute() {
        tasks.to_path_buf()
    } else {
        project_root.join(tasks)
    };
    let resolved = match std::fs::symlink_metadata(&candidate) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => new_task_root(&candidate)?,
        _ => canonical_existing(&candidate, "task root")?,
    };
    resolved.strip_prefix(project_root).map_err(|_| {
        anyhow!(
            "task root {} escapes project root {}",
            resolved.display(),
            project_root.display()
        )
    })?;
    Ok(resolved)
}

fn new_task_root(candidate: &Path) -> Result<PathBuf> {
    let (Some(parent), Some(std::path::Component::Normal(name))) =
        (candidate.parent(), candidate.components().next_back())
    else {
        return Err(anyhow!(
            "task root {} does not exist and names no directory to create",
            candidate.display()
        ));
    };
    let parent = parent
        .canonicalize().map(archon_shell::paths::plain)
        .ok()
        .filter(|parent| parent.is_dir())
        .ok_or_else(|| {
            anyhow!(
                "task root {} does not exist and its parent directory {} is not an existing directory; create the parent or name a task root under an existing directory",
                candidate.display(),
                parent.display()
            )
        })?;
    Ok(parent.join(name))
}

pub(crate) fn create_claimed_run(
    store: &WorkflowStore,
    task_root: &Path,
    spec: WorkflowSpec,
    state: &FixedDecompositionStateV1,
) -> Result<archon_workflow::WorkflowRun> {
    store
        .with_store_lock(|locked| {
            refuse_active_task_root(locked, task_root, None)
                .map_err(|error| archon_workflow::WorkflowError::PolicyDenied(error.to_string()))?;
            create_task_root_dir(task_root)?;
            let run = locked.create_run(spec)?;
            if let Err(error) =
                locked.write_run_json(&run.id, FIXED_DECOMPOSITION_STATE_PATH, state)
            {
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

/// Create the claimed task root when the launch named one that does not exist
/// yet. Its parent was resolved when the launch began; an existing directory
/// is left as it is.
fn create_task_root_dir(task_root: &Path) -> archon_workflow::WorkflowResult<()> {
    match std::fs::create_dir(task_root) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists && task_root.is_dir() => {
            Ok(())
        }
        Err(source) => Err(archon_workflow::WorkflowError::Io {
            path: task_root.to_path_buf(),
            source,
        }),
    }
}

fn refuse_active_task_root(
    store: &WorkflowStore,
    task_root: &Path,
    resuming: Option<&str>,
) -> Result<()> {
    let identity = path_text(task_root);
    for run in store.list_runs()? {
        if resuming == Some(run.id.as_str())
            || crate::command::workflow_task_root_reclaim::is_reclaimed(store, &run.id)?
            || is_released(store, &run)?
        {
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

/// Whether `run` gave its task root back when its launch failed before any
/// work started: the release names this run and its task root, and the run is
/// still the cancelled generation that wrote it.
fn is_released(store: &WorkflowStore, run: &archon_workflow::WorkflowRun) -> Result<bool> {
    let path = store.run_dir(&run.id).join(RELEASED_PATH);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    let record: serde_json::Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("parsing task-root release {}", path.display()))?;
    let state = read_fixed_state(store, &run.id)?;
    if record["schema_version"] != 1
        || record["run_id"] != run.id.as_str()
        || record["task_root"] != state.identity.task_root_identity.as_str()
    {
        return Err(anyhow!(
            "task-root release {} does not bind run {}",
            path.display(),
            run.id
        ));
    }
    Ok(run.status == archon_workflow::RunStatus::Cancelled
        && record["generation"] == run.generation)
}

/// Settle a launch that returned an error: cancel the run, and when no work
/// had started, give the task root back so the next decompose can claim it.
/// A run that started work keeps its claim, resumable until reclaimed.
pub(super) fn settle_launch_failure(
    store: &WorkflowStore,
    run_id: &str,
    launch_generation: u64,
    work_started: bool,
    error: anyhow::Error,
) -> anyhow::Error {
    if let Err(cleanup_error) = cancel_active_launch_failure(store, run_id, launch_generation) {
        return anyhow!(
            "fixed decomposition launch failed ({error:#}); terminal cleanup also failed ({cleanup_error})"
        );
    }
    if work_started {
        return error;
    }
    match release_unstarted_claim(store, run_id) {
        Ok(Some(task_root)) => anyhow!(
            "{error:#}; no work had started, so task root {task_root} was released: run workflow decompose again, or resume {run_id}"
        ),
        Ok(None) => error,
        Err(release_error) => anyhow!(
            "fixed decomposition launch failed ({error:#}); releasing its task root also failed ({release_error:#})"
        ),
    }
}

pub(crate) fn cancel_active_launch_failure(
    store: &WorkflowStore,
    run_id: &str,
    expected_generation: u64,
) -> Result<()> {
    let run = store.load_state(run_id)?;
    if run.generation != expected_generation {
        return Ok(());
    }
    if matches!(
        run.status,
        archon_workflow::RunStatus::Completed
            | archon_workflow::RunStatus::Paused
            | archon_workflow::RunStatus::Cancelled
            | archon_workflow::RunStatus::Failed
            | archon_workflow::RunStatus::Blocked
    ) {
        return Ok(());
    }
    archon_workflow::LifecycleController::new(store.clone())
        .apply(run_id, archon_workflow::LifecycleAction::Cancel)?;
    Ok(())
}

/// Record the release for a launch that failed before any work, when its run
/// is cancelled. Returns the released task root, or `None` when the run is in
/// any other state and so keeps whatever claim it has.
fn release_unstarted_claim(store: &WorkflowStore, run_id: &str) -> Result<Option<String>> {
    store
        .with_store_lock(|locked| {
            locked.with_run_lock(run_id, |locked| {
                let run = locked.load_state(run_id)?;
                if run.status != archon_workflow::RunStatus::Cancelled {
                    return Ok(None);
                }
                let state = read_fixed_state(locked, run_id).map_err(|error| {
                    archon_workflow::WorkflowError::StateCorrupt(format!("{error:#}"))
                })?;
                let task_root = state.identity.task_root_identity;
                locked.write_run_json(
                    run_id,
                    RELEASED_PATH,
                    &serde_json::json!({
                        "schema_version": 1,
                        "run_id": run_id,
                        "task_root": task_root,
                        "generation": run.generation,
                        "released_at": chrono::Utc::now().to_rfc3339(),
                        "reason": "the launch failed before any work started",
                    }),
                )?;
                Ok(Some(task_root))
            })
        })
        .map_err(Into::into)
}

/// Before a released run resumes, take its task root back: refused when
/// another run has claimed the root since, so one root never has two owners.
/// A run that was never released is left as it is.
pub(super) fn reclaim_released_task_root(store: &WorkflowStore, run_id: &str) -> Result<()> {
    store
        .with_store_lock(|locked| {
            let path = locked.run_dir(run_id).join(RELEASED_PATH);
            if !path.exists() {
                return Ok(());
            }
            let state = read_fixed_state(locked, run_id).map_err(|error| {
                archon_workflow::WorkflowError::StateCorrupt(format!("{error:#}"))
            })?;
            refuse_active_task_root(
                locked,
                Path::new(&state.identity.task_root_identity),
                Some(run_id),
            )
            .map_err(|error| archon_workflow::WorkflowError::PolicyDenied(error.to_string()))?;
            std::fs::remove_file(&path).map_err(|source| archon_workflow::WorkflowError::Io {
                path: path.clone(),
                source,
            })
        })
        .map_err(Into::into)
}

pub(super) fn read_fixed_state(
    store: &WorkflowStore,
    run_id: &str,
) -> Result<FixedDecompositionStateV1> {
    let value: serde_json::Value = read_run_json(store, run_id, FIXED_DECOMPOSITION_STATE_PATH)?;
    if value
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        != Some(u64::from(FIXED_DECOMPOSITION_STATE_SCHEMA_VERSION))
    {
        return Err(super::resume::upgrade::unmapped(
            "decomposition/state.json.schema_version",
            &format!(
                "found {}; this binary reads schema {}; install a compatible binary or explicitly migrate the state",
                value["schema_version"], FIXED_DECOMPOSITION_STATE_SCHEMA_VERSION
            ),
        ));
    }
    super::resume::upgrade::decode(value, FIXED_DECOMPOSITION_STATE_PATH)
}

pub(super) fn read_run_json<T: serde::de::DeserializeOwned>(
    store: &WorkflowStore,
    run_id: &str,
    relative: &str,
) -> Result<T> {
    let path = store.run_dir(run_id).join(relative);
    let value = serde_json::from_slice(
        &std::fs::read(&path)
            .map_err(|error| super::resume::upgrade::unmapped(relative, &error.to_string()))?,
    )
    .map_err(|error| super::resume::upgrade::unmapped(relative, &error.to_string()))?;
    super::resume::upgrade::decode(value, relative)
}
