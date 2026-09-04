//! Progress-aware waits for the live proof harnesses.
//!
//! A fixed clock on a phase boundary encodes a guess about how long the engine
//! needs there, and a real PRD's repair loop (several author, freeze and judge
//! rounds) outlived the guess while the run it was timing went on to complete.
//! These waits measure idleness instead: the clock restarts whenever the run's
//! durable event log grows, so only a run that has stopped producing events, or
//! has already ended without the awaited event, fails the wait. A hard cap still
//! bounds a run that keeps itself busy forever.
use super::*;
use std::time::{Duration, Instant};

/// How long a live run may go without appending a durable event before the
/// harness treats it as stuck. One provider call plus its host freeze and
/// batched judge fits inside this with room to spare; the engine emits no
/// heartbeat during a single provider call, so this is also the longest call
/// the harness tolerates.
pub const PROOF_IDLE_TIMEOUT: Duration = Duration::from_secs(1_500);
/// Absolute bound on one phase-boundary wait, regardless of progress.
pub const PROOF_PHASE_CAP: Duration = Duration::from_secs(4 * 3_600);
/// Absolute bound on a whole decomposition run, regardless of progress.
pub const PROOF_RUN_CAP: Duration = Duration::from_secs(8 * 3_600);

/// The statuses after which a run appends nothing more. The engine labels
/// every status transition `terminal_status`, including `paused` and
/// `running`, so the label alone must not end a wait.
pub const TERMINAL_STATUSES: [&str; 5] = [
    "completed",
    "needs_review",
    "blocked",
    "failed",
    "cancelled",
];

pub fn wait_for_event_line_while_progressing(
    project: &Path,
    run_id: &str,
    detail_markers: &[&str],
    idle_timeout: Duration,
    cap: Duration,
) -> Result<(), String> {
    let (kind, markers) = detail_markers
        .split_first()
        .ok_or_else(|| "typed event wait requires a kind".to_string())?;
    let path = archon_workflow::WorkflowStore::project(project).events_path(run_id);
    let mut clock = ProgressClock::start(idle_timeout, cap);
    loop {
        let (events, parse_error) = match parse_json_lines(&path) {
            Ok(events) => (events, None),
            Err(error) => (Vec::new(), Some(error)),
        };
        if let Some(verdict) = event_wait_verdict(&events, kind, markers) {
            return verdict;
        }
        clock.observe(log_len(&path));
        if let Some(stall) = clock.stalled() {
            let context = format!("waiting for {kind} {markers:?} on run {run_id}");
            return Err(match parse_error {
                Some(error) => format!("{stall} while {context}; event log unreadable: {error}"),
                None => format!("{stall} while {context}"),
            });
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

pub fn wait_for_terminal_run_while_progressing(
    project: &Path,
    run_id: &str,
    idle_timeout: Duration,
    cap: Duration,
) -> Result<archon_workflow::WorkflowRun, String> {
    let store = archon_workflow::WorkflowStore::project(project);
    let path = store.events_path(run_id);
    let mut clock = ProgressClock::start(idle_timeout, cap);
    loop {
        let run = store
            .load_state(run_id)
            .map_err(|error| error.to_string())?;
        let status = serde_json::to_value(&run.status)
            .ok()
            .and_then(|value| value.as_str().map(str::to_string))
            .unwrap_or_default();
        if TERMINAL_STATUSES.contains(&status.as_str()) {
            return Ok(run);
        }
        clock.observe(log_len(&path));
        if let Some(stall) = clock.stalled() {
            return Err(format!(
                "{stall} while waiting for run {run_id} to end (status {status})"
            ));
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// Pure decision over a snapshot of the event log: `Some(Ok)` when the awaited
/// event is present, `Some(Err)` when the run already ended without it, `None`
/// to keep waiting.
pub fn event_wait_verdict(
    events: &[serde_json::Value],
    kind: &str,
    markers: &[&str],
) -> Option<Result<(), String>> {
    let awaited = events.iter().any(|event| {
        event["kind"] == kind
            && markers
                .iter()
                .all(|marker| event["detail"].to_string().contains(marker))
    });
    if awaited {
        return Some(Ok(()));
    }
    events
        .iter()
        .rev()
        .filter(|event| event["detail"]["event"] == "terminal_status")
        .find_map(|event| event["detail"]["status"].as_str())
        .filter(|status| TERMINAL_STATUSES.contains(status))
        .map(|status| {
            Err(format!(
                "run ended with terminal status {status} before {kind} {markers:?}"
            ))
        })
}

/// An idle clock over a monotone progress measure, plus a hard cap. Progress is
/// the byte length of the append-only event log: it grows on every durable
/// event and stays readable even when a line is malformed.
pub struct ProgressClock {
    started: Instant,
    last_progress: Instant,
    seen: u64,
    idle_timeout: Duration,
    cap: Duration,
}

impl ProgressClock {
    pub fn start(idle_timeout: Duration, cap: Duration) -> Self {
        let now = Instant::now();
        Self {
            started: now,
            last_progress: now,
            seen: 0,
            idle_timeout,
            cap,
        }
    }

    pub fn observe(&mut self, progress: u64) {
        if progress > self.seen {
            self.seen = progress;
            self.last_progress = Instant::now();
        }
    }

    pub fn stalled(&self) -> Option<String> {
        let now = Instant::now();
        if now.duration_since(self.last_progress) >= self.idle_timeout {
            return Some(format!(
                "no progress for {}s ({} bytes of events seen)",
                self.idle_timeout.as_secs(),
                self.seen
            ));
        }
        if now.duration_since(self.started) >= self.cap {
            return Some(format!(
                "exceeded the {}s cap ({} bytes of events seen)",
                self.cap.as_secs(),
                self.seen
            ));
        }
        None
    }
}

fn log_len(path: &Path) -> u64 {
    std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
}

#[cfg(test)]
#[path = "workflow_decomposition_proof_progress_tests.rs"]
mod tests;
