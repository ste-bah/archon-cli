//! Whether a fixed host command generation still owns its run.

use archon_workflow::{RunStatus, WorkflowError, WorkflowResult, WorkflowRun, WorkflowStore};

use crate::command::workflow_host_command_supervisor::HostCommandSignal;

/// Narrow authority issued only after a reverify has authenticated an unused
/// prior report and acquired this run's executor lease.
#[derive(Debug, Clone)]
pub(crate) struct PausedRunAuthorization {
    pub(crate) invocation_id: String,
    pub(crate) expected_generation: u64,
    pub(crate) lease_pid: u32,
    pub(crate) lease_acquired_at: String,
}

impl PausedRunAuthorization {
    pub(crate) fn new(
        invocation_id: String,
        expected_generation: u64,
        lease: &crate::command::workflow_executor_lease::ExecutionLease,
    ) -> Self {
        Self {
            invocation_id,
            expected_generation,
            lease_pid: lease.holder().pid,
            lease_acquired_at: lease.holder().acquired_at.clone(),
        }
    }

    fn authorizes(
        &self,
        store: &WorkflowStore,
        run: &WorkflowRun,
        expected_generation: u64,
    ) -> bool {
        if self.expected_generation != expected_generation
            || run.generation != expected_generation
            || run.status != RunStatus::Paused
        {
            return false;
        }
        let events = store.events_path(&run.id);
        let invocation_started = archon_workflow::events::read_event_log(&events)
            .ok()
            .filter(|(_, damaged)| *damaged == 0)
            .is_some_and(|(events, _)| {
                events.iter().any(|event| {
                    event.kind == archon_workflow::WorkflowEventKind::DecompositionReverifyStarted
                        && event
                            .detail
                            .get("invocation_id")
                            .and_then(serde_json::Value::as_str)
                            == Some(self.invocation_id.as_str())
                })
            });
        let lease_path = store
            .run_dir(&run.id)
            .join(crate::command::workflow_executor_lease::LEASE);
        let lease_matches = crate::command::workflow_executor_lease::is_held_by(
            &lease_path,
            self.lease_pid,
            &self.lease_acquired_at,
        );
        invocation_started && lease_matches
    }
}

/// Fails unless `expected_generation` still owns a running run, with the
/// run's actual control decision: a paused run reports a pause (so the call
/// is recorded interrupted as paused), a cancelled or superseded one a
/// cancellation. Checked before every attempt and before publication.
/// Issue 337: a deliberate stop of this generation ended the run (a cancel
/// of the call), and an unreadable stop record pauses it
/// (`control_pause::require_no_terminal_stop`).
#[cfg(test)]
pub(crate) fn require_run_owned(
    store: &WorkflowStore,
    run_id: &str,
    expected_generation: u64,
) -> WorkflowResult<()> {
    require_run_owned_authorized(store, run_id, expected_generation, None)
}

pub(crate) fn require_run_owned_authorized(
    store: &WorkflowStore,
    run_id: &str,
    expected_generation: u64,
    authorization: Option<&PausedRunAuthorization>,
) -> WorkflowResult<()> {
    let run = store.load_state(run_id)?;
    if authorized_owned(store, &run, expected_generation, authorization)? {
        return archon_workflow::control_pause::require_no_terminal_stop(
            store,
            &run,
            &caller(expected_generation),
        );
    }
    owned(&run, expected_generation)?;
    archon_workflow::control_pause::require_no_terminal_stop(
        store,
        &run,
        &caller(expected_generation),
    )
}

/// [`require_run_owned`] for a caller that holds the run lock (the parent
/// publication).
#[cfg(test)]
pub(crate) fn require_run_owned_locked(
    locked: &WorkflowStore,
    run_id: &str,
    expected_generation: u64,
) -> WorkflowResult<()> {
    require_run_owned_locked_authorized(locked, run_id, expected_generation, None)
}

pub(crate) fn require_run_owned_locked_authorized(
    locked: &WorkflowStore,
    run_id: &str,
    expected_generation: u64,
    authorization: Option<&PausedRunAuthorization>,
) -> WorkflowResult<()> {
    let run = locked.load_state(run_id)?;
    if authorized_owned(locked, &run, expected_generation, authorization)? {
        return archon_workflow::control_pause::require_no_terminal_stop_locked(
            locked,
            &run,
            &caller(expected_generation),
        );
    }
    owned(&run, expected_generation)?;
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
pub(crate) fn supervisor_signal_authorized(
    store: &WorkflowStore,
    run: &WorkflowRun,
    expected_generation: u64,
    authorization: Option<&PausedRunAuthorization>,
) -> Option<HostCommandSignal> {
    let owned = (|| {
        if authorized_owned(store, run, expected_generation, authorization)? {
            return archon_workflow::control_pause::require_no_terminal_stop(
                store,
                run,
                "a host command in flight",
            );
        }
        owned(run, expected_generation)?;
        archon_workflow::control_pause::require_no_terminal_stop(
            store,
            run,
            "a host command in flight",
        )
    })();
    match owned {
        Ok(()) => None,
        Err(WorkflowError::ControlPaused(_)) => Some(HostCommandSignal::Paused),
        Err(_) => Some(HostCommandSignal::Cancelled),
    }
}

fn authorized_owned(
    store: &WorkflowStore,
    run: &WorkflowRun,
    expected_generation: u64,
    authorization: Option<&PausedRunAuthorization>,
) -> WorkflowResult<bool> {
    if run.generation != expected_generation {
        return Ok(false);
    }
    Ok(run.status == RunStatus::Paused
        && authorization
            .is_some_and(|authorization| authorization.authorizes(store, run, expected_generation)))
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
