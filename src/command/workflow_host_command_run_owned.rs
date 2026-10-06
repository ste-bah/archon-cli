//! Whether a fixed host command generation still owns its run.

use archon_workflow::{RunStatus, WorkflowError, WorkflowResult, WorkflowRun, WorkflowStore};

/// Fails unless `expected_generation` still owns a running run, with the
/// run's actual control decision: a paused run reports a pause (so the call
/// is recorded interrupted as paused), a cancelled or superseded one a
/// cancellation. Checked before every attempt and before publication.
/// Issue 337: a deliberate stop of this generation ended the run (a cancel
/// of the call), and an unreadable stop record pauses it
/// (`control_pause::require_no_terminal_stop`).
pub(crate) fn require_run_owned(
    store: &WorkflowStore,
    run_id: &str,
    expected_generation: u64,
) -> WorkflowResult<()> {
    let run = owned_run(store, run_id, expected_generation)?;
    archon_workflow::control_pause::require_no_terminal_stop(
        store,
        &run,
        &caller(expected_generation),
    )
}

/// [`require_run_owned`] for a caller that holds the run lock (the parent
/// publication).
pub(crate) fn require_run_owned_locked(
    locked: &WorkflowStore,
    run_id: &str,
    expected_generation: u64,
) -> WorkflowResult<()> {
    let run = owned_run(locked, run_id, expected_generation)?;
    archon_workflow::control_pause::require_no_terminal_stop_locked(
        locked,
        &run,
        &caller(expected_generation),
    )
}

fn caller(expected_generation: u64) -> String {
    format!("fixed HostCommand generation {expected_generation}")
}

fn owned_run(
    store: &WorkflowStore,
    run_id: &str,
    expected_generation: u64,
) -> WorkflowResult<WorkflowRun> {
    let run = store.load_state(run_id)?;
    match run.status {
        RunStatus::Paused => Err(WorkflowError::ControlPaused(format!(
            "run {run_id} is paused; fixed HostCommand generation {expected_generation} stops"
        ))),
        RunStatus::Cancelled => Err(WorkflowError::ControlCancelled(format!(
            "run {run_id} is cancelled; fixed HostCommand generation {expected_generation} stops"
        ))),
        _ if run.generation != expected_generation => {
            Err(WorkflowError::ControlCancelled(format!(
                "fixed HostCommand generation {expected_generation} no longer owns run {run_id}; current generation is {}",
                run.generation
            )))
        }
        _ => Ok(run),
    }
}
