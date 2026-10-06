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
    generation_owns(&store.load_state(run_id)?, generation)
}

/// [`require_generation`] on a loaded `run`.
fn generation_owns(run: &WorkflowRun, generation: u64) -> WorkflowResult<()> {
    let run_id = &run.id;
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

/// Who pauses a run, and the ownership the pause requires (Issue 316).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseOwner {
    /// Exactly this generation still owns the run: any change since -- an
    /// edit too -- supersedes the caller, which stops. For a script-host
    /// call, which re-dispatches a call an edit superseded.
    Generation(u64),
    /// The executor launched at this generation still owns the run
    /// ([`require_executor`]). An edit that kept the executor (a restart of
    /// a stage or item, a force-accept) moved the generation on but refuses
    /// nothing; a resume that replaced the executor refuses. The pause takes
    /// the run's generation at the pause. For the run end of an executor.
    Executor(u64),
    /// No executor to check (an unbound session): the run's generation at
    /// the pause.
    Unfenced,
}

impl PauseOwner {
    /// The owner of a session bound to `executor` (unbound: [`Self::Unfenced`]).
    pub fn of_executor(executor: Option<u64>) -> Self {
        executor.map_or(Self::Unfenced, Self::Executor)
    }

    /// Fails unless this owner may still write for `run`: its executor owns
    /// the run (a generation owner, the executor it was dispatched under).
    pub fn require_writer(&self, run: &WorkflowRun) -> WorkflowResult<()> {
        match *self {
            Self::Generation(generation) | Self::Executor(generation) => {
                require_executor(run, generation)
            }
            Self::Unfenced => Ok(()),
        }
    }

    /// Fails unless this owner may pause `run` now: the run is running and
    /// owned as the variant requires.
    pub fn require_pauser(&self, run: &WorkflowRun) -> WorkflowResult<()> {
        if let Self::Generation(generation) = *self {
            return generation_owns(run, generation);
        }
        match run.status {
            RunStatus::Paused => Err(WorkflowError::ControlPaused(format!(
                "run {} is paused; {self:?} stops",
                run.id
            ))),
            RunStatus::Cancelled => Err(WorkflowError::ControlCancelled(format!(
                "run {} is cancelled; {self:?} stops",
                run.id
            ))),
            _ => self.require_writer(run),
        }
    }
}

/// Pauses `run_id`, owned by `generation`, with `detail` as the evidence its
/// `Paused` event carries. The outer result is the transition (an error when
/// `generation` no longer owns the run, and nothing changed); the inner one
/// is the evidence event, whose failure never undoes the pause.
pub fn pause_with_evidence(
    store: &WorkflowStore,
    run_id: &str,
    generation: u64,
    detail: serde_json::Value,
) -> WorkflowResult<WorkflowResult<u64>> {
    pause_owned(store, run_id, PauseOwner::Generation(generation), detail)
}

/// [`pause_with_evidence`] for `owner` (Issue 316): the ownership check and
/// the pause in ONE run-lock critical section, so no change of owner or
/// generation falls between them. A pause owned by an executor takes the
/// run's generation at the pause.
pub fn pause_owned(
    store: &WorkflowStore,
    run_id: &str,
    owner: PauseOwner,
    mut detail: serde_json::Value,
) -> WorkflowResult<WorkflowResult<u64>> {
    let owned = store.with_run_lock(run_id, |locked| {
        let mut run = locked.load_state(run_id)?;
        owner.require_pauser(&run)?;
        // Issue 337: a call of the generation a deliberate stop ended never
        // pauses it; a run end, after its script settled, still may.
        let unreadable_stop = terminal_stop_before_pause(
            locked,
            &run,
            "its pause",
            matches!(owner, PauseOwner::Executor(_)),
        )?;
        let paused_by = run.generation;
        apply_pause(&mut run);
        locked.save_state(&run)?;
        if let Some(object) = detail.as_object_mut() {
            object.insert("action".into(), "pause".into());
            object.insert("generation".into(), run.generation.into());
            object.insert("paused_by_generation".into(), paused_by.into());
            if let PauseOwner::Executor(executor) = owner {
                object.insert("paused_by_executor".into(), executor.into());
            }
            if let Some(evidence) = unreadable_stop {
                object.insert("terminal_stop_unreadable".into(), evidence);
            }
        }
        // The run is paused from here whatever happens to the evidence.
        Ok(emit(locked, run_id, detail))
    });
    if let Err(refused) = &owned {
        tracing::warn!(
            run_id,
            ?owner,
            %refused,
            "stall pause refused: its owner no longer owns the run"
        );
    }
    owned
}

/// Where a run's validated terminal stop is recorded, relative to its run
/// directory (Issue 337).
pub const TERMINAL_STOP_RECORD: &str = "v2/terminal-stop.json";

/// A deliberate terminal stop the script host validated, recorded under the
/// run lock at the generation it ended (Issue 337).
///
/// The run stays `Running`, at that generation, until the stopping executor
/// finalizes it, so the run state alone cannot tell a call still in flight
/// that the run has ended. This record does: while it binds the run's
/// generation, no call of that generation pauses the run (a pause would leave
/// the deliberate verdict resumable), no host command publishes into it, and
/// a host command supervisor ends its process. A lifecycle edit -- an
/// operator's pause or cancel -- moves the generation and outranks it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TerminalStopRecord {
    pub generation: u64,
    pub reason: String,
}

/// Records `reason` as the terminal stop of `run` at its generation. The
/// caller holds the run lock and has checked that it owns the run.
pub fn record_terminal_stop(
    store: &WorkflowStore,
    run: &WorkflowRun,
    reason: &str,
) -> WorkflowResult<()> {
    let record = TerminalStopRecord {
        generation: run.generation,
        reason: reason.to_string(),
    };
    store.write_run_json(&run.id, TERMINAL_STOP_RECORD, &record)
}

/// What a run's terminal stop record says for `run` now (Issue 337).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalStopState {
    /// No stop binds the run: none recorded, or one a lifecycle edit made
    /// stale, or the run no longer runs.
    Clear,
    /// A stop recorded at the run's current generation while it runs.
    InForce(TerminalStopRecord),
    /// The record exists but cannot be read: it may hold a stop.
    Unreadable(String),
}

/// Reads the stop record for `run`; reports nothing (see the callers).
pub fn terminal_stop_state(store: &WorkflowStore, run: &WorkflowRun) -> TerminalStopState {
    if !matches!(run.status, RunStatus::Planned | RunStatus::Running) {
        return TerminalStopState::Clear;
    }
    let path = store.run_dir(&run.id).join(TERMINAL_STOP_RECORD);
    let read = match std::fs::read(&path) {
        Ok(raw) => serde_json::from_slice::<TerminalStopRecord>(&raw).map_err(|e| e.to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return TerminalStopState::Clear;
        }
        Err(error) => Err(error.to_string()),
    };
    match read {
        Ok(record) if record.generation == run.generation => TerminalStopState::InForce(record),
        Ok(_) => TerminalStopState::Clear,
        Err(error) => TerminalStopState::Unreadable(error),
    }
}

fn stopped_refusal(run: &WorkflowRun, stop: &TerminalStopRecord, caller: &str) -> WorkflowError {
    WorkflowError::ControlCancelled(format!(
        "run {} stopped terminally at generation {}: {}; {caller} stops",
        run.id, stop.generation, stop.reason
    ))
}

/// Moves an unreadable record aside, kept as evidence, and reports it ONCE:
/// no later reader finds it. The caller holds the run lock and pauses the
/// run with the evidence returned.
fn set_aside_unreadable(
    locked: &WorkflowStore,
    run: &WorkflowRun,
    error: &str,
) -> serde_json::Value {
    let dir = locked.run_dir(&run.id);
    let kept = format!("{TERMINAL_STOP_RECORD}.unreadable-g{}", run.generation);
    let moved = std::fs::rename(dir.join(TERMINAL_STOP_RECORD), dir.join(&kept));
    tracing::warn!(
        run_id = %run.id,
        error,
        kept = %kept,
        moved = moved.is_ok(),
        "terminal stop record unreadable: the run pauses, the record is kept"
    );
    serde_json::json!({
        "event": "terminal_stop_unreadable",
        "error": error,
        "kept_at": moved.as_ref().ok().map(|_| kept.clone()),
        "set_aside_error": moved.err().map(|e| e.to_string()),
    })
}

/// For a pause the caller takes under the run lock. A stop in force refuses
/// the pause of a call of its generation (a run end, `run_end`, still
/// pauses). An unreadable record is set aside; its evidence is returned for
/// the pause to carry.
pub fn terminal_stop_before_pause(
    locked: &WorkflowStore,
    run: &WorkflowRun,
    caller: &str,
    run_end: bool,
) -> WorkflowResult<Option<serde_json::Value>> {
    match terminal_stop_state(locked, run) {
        TerminalStopState::Clear => Ok(None),
        TerminalStopState::InForce(stop) if !run_end => Err(stopped_refusal(run, &stop, caller)),
        TerminalStopState::InForce(_) => Ok(None),
        TerminalStopState::Unreadable(error) => Ok(Some(set_aside_unreadable(locked, run, &error))),
    }
}

/// For a writer of `run` (a host command's start or publication); the caller
/// holds the run lock and loaded `run` under it. A stop in force refuses it
/// as a cancel. An unreadable record may hold a stop, so the writer never
/// continues past it: the run pauses here with that evidence (the record set
/// aside), and the writer is refused with the pause.
pub fn require_no_terminal_stop_locked(
    locked: &WorkflowStore,
    run: &WorkflowRun,
    caller: &str,
) -> WorkflowResult<()> {
    match terminal_stop_state(locked, run) {
        TerminalStopState::Clear => Ok(()),
        TerminalStopState::InForce(stop) => Err(stopped_refusal(run, &stop, caller)),
        TerminalStopState::Unreadable(error) => {
            let mut detail = set_aside_unreadable(locked, run, &error);
            let mut paused = run.clone();
            apply_pause(&mut paused);
            locked.save_state(&paused)?;
            detail["action"] = "pause".into();
            detail["generation"] = paused.generation.into();
            detail["caller"] = caller.into();
            if let Err(error) = emit(locked, &run.id, detail) {
                tracing::warn!(%error, run_id = %run.id, "unreadable stop pause event not recorded");
            }
            Err(WorkflowError::ControlPaused(format!(
                "run {} is paused: its terminal stop record could not be read ({error}), so it may hold a deliberate stop; the record is kept beside it as {TERMINAL_STOP_RECORD}.unreadable-g{}. Inspect it, then resume; {caller} stops",
                run.id, run.generation
            )))
        }
    }
}

/// [`require_no_terminal_stop_locked`] for a caller without the run lock:
/// the lock is taken only when the record is not clear.
pub fn require_no_terminal_stop(
    store: &WorkflowStore,
    run: &WorkflowRun,
    caller: &str,
) -> WorkflowResult<()> {
    match terminal_stop_state(store, run) {
        TerminalStopState::Clear => Ok(()),
        TerminalStopState::InForce(stop) => Err(stopped_refusal(run, &stop, caller)),
        TerminalStopState::Unreadable(_) => store.with_run_lock(&run.id, |locked| {
            let current = locked.load_state(&run.id)?;
            // Moved on meanwhile: the caller's own ownership check decides.
            if current.generation != run.generation {
                return Ok(());
            }
            require_no_terminal_stop_locked(locked, &current, caller)
        }),
    }
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
#[path = "control_pause_terminal_stop_tests.rs"]
mod terminal_stop_tests;
#[cfg(test)]
#[path = "control_pause_tests.rs"]
mod tests;
