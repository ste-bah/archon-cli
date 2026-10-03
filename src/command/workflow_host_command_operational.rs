//! Operational outcomes of a trusted host command: limits it hit, never
//! verdicts on its work (Issue #255).
//!
//! # Outcome contract
//!
//! The fixed executor tells three endings of a host command apart:
//!
//! * **Completed** - any exit status the command chose, except the one below.
//!   Exit 0 goes on to publication; any other code is the command's own verdict
//!   and reaches the script unchanged.
//! * **Timed out** - the supervisor killed the process group at the catalog's
//!   `timeout_secs` (`SupervisedProcessOutput::timed_out`).
//! * **Incomplete, resumable** - the command exited with
//!   [`EXIT_INCOMPLETE_RESUMABLE`] (75, `EX_TEMPFAIL` in `sysexits.h`). It
//!   stopped before it finished, on an operational limit of its own (for
//!   example an internal deadline set below the catalog wall clock). It
//!   persisted the work it finished OUTSIDE its call staging directory, and a
//!   re-run of the same call continues from there. Its stdout is ignored and
//!   nothing it staged is published.
//!
//! Either operational ending may report progress with a stderr line
//! `archon-host-progress: <n>` ([`PROGRESS_MARKER`]). `<n>` is the cumulative
//! count of units of work persisted for this call, so it does not decrease
//! from one attempt to the next. The last such line of an attempt counts.
//!
//! # Policy
//!
//! An operational ending is retried in place, the same call with the same
//! input, at most [`MAX_OPERATIONAL_RETRIES`] times and only while the
//! reported progress grows. Growth needs a baseline: until one attempt has
//! reported progress and a later one reports more, the call is treated as
//! having no marker, which allows exactly one retry. The call staging directory is cleared before every attempt. When
//! the policy stops, the run is PAUSED, never failed: the call is recorded as
//! interrupted, so a resume runs it again instead of reusing it.

use std::path::Path;

use archon_workflow::{
    RunStatus, StageStatus, WorkflowError, WorkflowEventKind, WorkflowEventLog, WorkflowResult,
    WorkflowStore,
};

use super::workflow_host_command_supervisor::SupervisedProcessOutput;

/// The exit status of an incomplete, resumable host command (`EX_TEMPFAIL`).
pub(crate) const EXIT_INCOMPLETE_RESUMABLE: i32 = 75;
/// The stderr line prefix that reports persisted progress.
pub(crate) const PROGRESS_MARKER: &str = "archon-host-progress:";
/// Retries of one call after its first operational ending.
pub(crate) const MAX_OPERATIONAL_RETRIES: u32 = 2;

/// The stderr line a host command writes to report `completed` units.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn progress_line(completed: u64) -> String {
    format!("{PROGRESS_MARKER} {completed}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OperationalKind {
    TimedOut,
    IncompleteResumable,
}

impl OperationalKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::TimedOut => "timed_out",
            Self::IncompleteResumable => "incomplete_resumable",
        }
    }
}

/// The operational ending of `output`, or `None` when the command completed.
pub(crate) fn classify(output: &SupervisedProcessOutput) -> Option<OperationalKind> {
    if output.timed_out {
        Some(OperationalKind::TimedOut)
    } else if output.exit_code == Some(EXIT_INCOMPLETE_RESUMABLE) {
        Some(OperationalKind::IncompleteResumable)
    } else {
        None
    }
}

/// The progress the last [`PROGRESS_MARKER`] line of `stderr` reports.
pub(crate) fn reported_progress(stderr: &[u8]) -> Option<u64> {
    String::from_utf8_lossy(stderr)
        .lines()
        .rev()
        .find_map(|line| {
            line.trim()
                .strip_prefix(PROGRESS_MARKER)?
                .trim()
                .parse()
                .ok()
        })
}

/// One attempt that ended operationally: the evidence each event carries.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(crate) struct OperationalAttempt {
    pub(crate) attempt: u32,
    pub(crate) reason: &'static str,
    pub(crate) elapsed_secs: u64,
    pub(crate) progress: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NextStep {
    Retry,
    Pause(&'static str),
}

/// What to do after the last attempt of `history` ended operationally.
pub(crate) fn next_step(history: &[OperationalAttempt]) -> NextStep {
    let Some((last, earlier)) = history.split_last() else {
        return NextStep::Retry;
    };
    if earlier.len() as u64 >= u64::from(MAX_OPERATIONAL_RETRIES) {
        return NextStep::Pause("retries_exhausted");
    }
    let before = earlier.iter().filter_map(|a| a.progress).max();
    match (last.progress, before) {
        (Some(now), Some(before)) if now > before => NextStep::Retry,
        (Some(_), Some(_)) => NextStep::Pause("no_progress"),
        // No baseline to measure growth against: the no-marker allowance.
        _ if earlier.is_empty() => NextStep::Retry,
        _ => NextStep::Pause("no_progress_evidence"),
    }
}

/// The call an operational ending belongs to.
pub(crate) struct OperationalReport<'a> {
    pub(crate) run_id: &'a str,
    pub(crate) call_id: &'a str,
    pub(crate) command_id: &'a str,
    pub(crate) limit_secs: u64,
    pub(crate) attempts: &'a [OperationalAttempt],
}

impl OperationalReport<'_> {
    fn resume_command(&self) -> String {
        format!("archon workflow resume --live --yes {}", self.run_id)
    }

    fn last(&self) -> Option<&OperationalAttempt> {
        self.attempts.last()
    }
}

/// Fails unless `expected_generation` still owns a running run, with the
/// run's actual control decision: a paused run reports a pause (so the call
/// is recorded interrupted as paused), a cancelled or superseded one a
/// cancellation. Checked before every attempt and before publication.
pub(crate) fn require_run_owned(
    store: &WorkflowStore,
    run_id: &str,
    expected_generation: u64,
) -> WorkflowResult<()> {
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
        _ => Ok(()),
    }
}

/// Records that the last attempt of `report` ended operationally and the call
/// runs again. Evidence only: a failure to record never stops the retry.
pub(crate) fn record_retry(store: &WorkflowStore, run_root: &Path, report: &OperationalReport) {
    let Some(last) = report.last() else {
        return;
    };
    let detail = serde_json::json!({
        "event": "host_command_operational_retry",
        "call_id": report.call_id,
        "command_id": report.command_id,
        "attempt": last.attempt,
        "next_attempt": last.attempt.saturating_add(1),
        "reason": last.reason,
        "elapsed_secs": last.elapsed_secs,
        "limit_secs": report.limit_secs,
        "progress": last.progress,
    });
    tracing::warn!(
        call_id = report.call_id,
        command_id = report.command_id,
        attempt = last.attempt,
        reason = last.reason,
        elapsed_secs = last.elapsed_secs,
        limit_secs = report.limit_secs,
        progress = ?last.progress,
        "host command ended on an operational limit; retrying the same call"
    );
    match emit(
        store,
        report.run_id,
        WorkflowEventKind::StageStalled,
        detail,
    ) {
        Ok(seq) => append_log(
            run_root,
            &format!(
                "event_id={seq} transition=host_command_operational_retry call_id={} command_id={} attempt={} reason={} elapsed_secs={} limit_secs={} progress={}",
                report.call_id,
                report.command_id,
                last.attempt,
                last.reason,
                last.elapsed_secs,
                report.limit_secs,
                progress_text(last.progress)
            ),
        ),
        Err(error) => tracing::warn!(%error, "host command retry event not recorded"),
    }
}

/// Pauses the run because `report`'s call stopped on an operational limit,
/// and returns the control error the call must end with. It is the same
/// transition `workflow pause` makes (status, running stages, generation), so
/// the script host records the call as interrupted and a resume re-runs it.
pub(crate) fn pause_run(
    store: &WorkflowStore,
    run_root: &Path,
    expected_generation: u64,
    report: &OperationalReport,
    cause: &'static str,
) -> WorkflowError {
    let last = report.last();
    let reason = last.map_or("unknown", |attempt| attempt.reason);
    let message = format!(
        "host command '{}' (call {}) stopped on an operational limit ({reason}, limit {}s) after {} attempt(s), cause {cause}; the run is paused, not failed, and the call runs again on resume: {}",
        report.command_id,
        report.call_id,
        report.limit_secs,
        report.attempts.len(),
        report.resume_command()
    );
    let paused = store.with_run_lock(report.run_id, |locked| {
        require_run_owned(locked, report.run_id, expected_generation)?;
        let mut run = locked.load_state(report.run_id)?;
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
        locked.save_state(&run)?;
        let detail = serde_json::json!({
            "action": "pause",
            "event": "host_command_operational_pause",
            "generation": run.generation,
            "call_id": report.call_id,
            "command_id": report.command_id,
            "reason": reason,
            "cause": cause,
            "limit_secs": report.limit_secs,
            "attempts": report.attempts,
            "resume": report.resume_command(),
        });
        // The run is paused from here whatever happens to the evidence.
        Ok(emit(
            locked,
            report.run_id,
            WorkflowEventKind::Paused,
            detail,
        ))
    });
    match paused {
        Ok(event) => {
            tracing::warn!(run_id = report.run_id, "{message}");
            match event {
                Ok(seq) => append_log(
                    run_root,
                    &format!(
                        "event_id={seq} transition=host_command_operational_pause call_id={} command_id={} reason={reason} cause={cause} limit_secs={} attempts={} next_action=resume run_id={}",
                        report.call_id,
                        report.command_id,
                        report.limit_secs,
                        report.attempts.len(),
                        report.run_id
                    ),
                ),
                Err(error) => tracing::warn!(%error, "host command pause event not recorded"),
            }
            WorkflowError::ControlPaused(message)
        }
        Err(error) => error,
    }
}

fn progress_text(progress: Option<u64>) -> String {
    progress.map_or_else(|| "none".to_string(), |value| value.to_string())
}

fn emit(
    store: &WorkflowStore,
    run_id: &str,
    kind: WorkflowEventKind,
    detail: serde_json::Value,
) -> WorkflowResult<u64> {
    let seq = store.next_event_seq(run_id)?;
    WorkflowEventLog::new(store.clone()).emit(
        run_id,
        seq,
        kind,
        archon_workflow::events::sanitize_value(detail),
    )?;
    Ok(seq)
}

/// Appends `line` to the fixed decomposition's operator log, when the run is
/// one. Evidence only: a missing or unreadable log never changes the outcome.
fn append_log(run_root: &Path, line: &str) {
    let state_path = run_root.join(super::workflow_decompose_state::FIXED_STATE_PATH);
    let Ok(raw) = std::fs::read(&state_path) else {
        return;
    };
    let appended = serde_json::from_slice::<archon_workflow::FixedDecompositionStateV1>(&raw)
        .map_err(WorkflowError::from)
        .and_then(|state| {
            super::workflow_decompose_log::validated_fixed_log_path(
                Path::new(&state.log_path),
                &state.identity,
            )
        })
        .and_then(|path| super::workflow_decompose_log::append_nofollow_line(&path, line));
    if let Err(error) = appended {
        tracing::warn!(%error, "host command operational log line not written");
    }
}

#[cfg(test)]
#[path = "workflow_host_command_operational_tests.rs"]
mod tests;
