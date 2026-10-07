//! Whether a fixed host command generation still owns its run.

use archon_workflow::{RunStatus, WorkflowError, WorkflowResult, WorkflowRun, WorkflowStore};

use crate::command::workflow_host_command_supervisor::HostCommandSignal;

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

/// The signal a host command supervisor sends its child once a poll read
/// `run`: none while `expected_generation` still owns it, the pause of a
/// paused run, and a cancellation of a cancelled or superseded one. Issue
/// 337: a deliberate stop of this generation cancels the call, and an
/// unreadable stop record pauses it -- the checks [`require_run_owned`]
/// makes before every attempt, on the state the poll already read.
pub(crate) fn supervisor_signal(
    store: &WorkflowStore,
    run: &WorkflowRun,
    expected_generation: u64,
) -> Option<HostCommandSignal> {
    let owned = owned(run, expected_generation).and_then(|()| {
        archon_workflow::control_pause::require_no_terminal_stop(
            store,
            run,
            "a host command in flight",
        )
    });
    match owned {
        Ok(()) => None,
        Err(WorkflowError::ControlPaused(_)) => Some(HostCommandSignal::Paused),
        Err(_) => Some(HostCommandSignal::Cancelled),
    }
}

fn owned_run(
    store: &WorkflowStore,
    run_id: &str,
    expected_generation: u64,
) -> WorkflowResult<WorkflowRun> {
    let run = store.load_state(run_id)?;
    owned(&run, expected_generation)?;
    Ok(run)
}

fn owned(run: &WorkflowRun, expected_generation: u64) -> WorkflowResult<()> {
    let run_id = &run.id;
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
        _ => Ok(()),
    }
}
