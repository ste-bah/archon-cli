//! Pausing a run on a stall, with evidence (Issues 255, 262, 263).
//!
//! The one transition every no-progress bound uses instead of failing a run:
//! the same change `workflow pause` makes (status, running stages and items,
//! generation), plus a `Paused` event whose detail is the evidence. Whatever
//! was in flight is recorded interrupted and runs again on resume.
//!
//! The pause belongs to the generation that observed the stall. When the run
//! is no longer owned by it -- the operator paused, resumed or restarted it
//! meanwhile -- nothing changes: the caller is obsolete and stops, and the
//! newer owner keeps the run.

use crate::error::{WorkflowError, WorkflowResult};
use crate::events::{WorkflowEventKind, WorkflowEventLog};
use crate::run::{RunStatus, StageStatus, WorkflowRun};
use crate::store::WorkflowStore;

/// Fails unless `generation` still owns a running run, with the run's own
/// control decision: a paused run reports a pause, a cancelled or newer one a
/// cancellation of the caller.
pub fn require_generation(
    store: &WorkflowStore,
    run_id: &str,
    generation: u64,
) -> WorkflowResult<()> {
    let run = store.load_state(run_id)?;
    match run.status {
        RunStatus::Paused => Err(WorkflowError::ControlPaused(format!(
            "run {run_id} is paused; generation {generation} stops"
        ))),
        RunStatus::Cancelled => Err(WorkflowError::ControlCancelled(format!(
            "run {run_id} is cancelled; generation {generation} stops"
        ))),
        _ if run.generation != generation => Err(WorkflowError::ControlCancelled(format!(
            "generation {generation} no longer owns run {run_id}; current generation is {}",
            run.generation
        ))),
        _ => Ok(()),
    }
}

/// Fails unless the executor launched at `generation` still owns `run`:
/// a resume or takeover since gave the run to a newer executor, and the
/// caller is a stale session that may change nothing. Edits that move the
/// generation but keep the executor (a pause, a restart of a running stage)
/// keep it the owner. The caller holds the run lock.
pub fn require_executor(run: &WorkflowRun, generation: u64) -> WorkflowResult<()> {
    if run.execution_owned_at(generation) {
        return Ok(());
    }
    Err(WorkflowError::ControlCancelled(format!(
        "executor generation {generation} no longer owns run {}; current generation is {} (executor generation {}); the stale session changes nothing",
        run.id,
        run.generation,
        run.executor_generation
            .map_or_else(|| "unrecorded".to_string(), |owner| owner.to_string()),
    )))
}

/// The transition `workflow pause` makes: the run, its running stages and
/// items paused, and the generation moved on. The caller holds the run lock,
/// has checked ownership, and saves the run.
pub fn apply_pause(run: &mut WorkflowRun) {
    run.status = RunStatus::Paused;
    for stage in run.stages.values_mut() {
        if stage.status == StageStatus::Running {
            stage.status = StageStatus::Paused;
            stage.completed_at = None;
        }
    }
    for item in run.items.values_mut() {
        if item.status == StageStatus::Running {
            item.status = StageStatus::Paused;
        }
    }
    run.generation = run.generation.saturating_add(1);
    run.mark_updated();
}

/// Pauses `run_id`, owned by `generation`, with `detail` as the evidence its
/// `Paused` event carries. The outer result is the transition (an error when
/// `generation` no longer owns the run, and nothing changed); the inner one
/// is the evidence event, whose failure never undoes the pause.
pub fn pause_with_evidence(
    store: &WorkflowStore,
    run_id: &str,
    generation: u64,
    mut detail: serde_json::Value,
) -> WorkflowResult<WorkflowResult<u64>> {
    let owned = store.with_run_lock(run_id, |locked| {
        require_generation(locked, run_id, generation)?;
        let mut run = locked.load_state(run_id)?;
        apply_pause(&mut run);
        locked.save_state(&run)?;
        if let Some(object) = detail.as_object_mut() {
            object.insert("action".into(), "pause".into());
            object.insert("generation".into(), run.generation.into());
            object.insert("paused_by_generation".into(), generation.into());
        }
        // The run is paused from here whatever happens to the evidence.
        Ok(emit(locked, run_id, detail))
    });
    if let Err(refused) = &owned {
        tracing::warn!(
            run_id,
            generation,
            %refused,
            "stall pause refused: the generation that observed it no longer owns the run"
        );
    }
    owned
}

fn emit(store: &WorkflowStore, run_id: &str, detail: serde_json::Value) -> WorkflowResult<u64> {
    let seq = store.next_event_seq(run_id)?;
    WorkflowEventLog::new(store.clone()).emit(
        run_id,
        seq,
        WorkflowEventKind::Paused,
        crate::events::sanitize_value(detail),
    )?;
    Ok(seq)
}

#[cfg(test)]
#[path = "control_pause_tests.rs"]
mod tests;
