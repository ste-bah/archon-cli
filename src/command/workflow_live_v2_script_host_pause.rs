//! `w.pause(id, evidence)`: a script asks the host to pause its run (Issue 261).
//!
//! A script loop that stops making progress has two honest endings: fail the
//! run, or pause it so an operator or a later fix can change something and
//! resume. Failing is the wrong one for long work -- a failed run cannot be
//! resumed -- and before this a script had no way to ask for the other: a
//! pause reached it only from outside, as an error.
//!
//! # Contract
//!
//! The first request for a pause id PAUSES the run: the same transition
//! `workflow pause` makes (`control_pause::apply_pause`: status, running
//! stages and items, generation), with one evidence event carrying the
//! script's evidence. Only the executor that owns the run may ask: a session
//! a restart or a resume has replaced is refused as stale
//! (`control_pause::require_executor`), and nothing changes. The call then ends
//! with the pause error, like any call a pause stops, and the script unwinds.
//! A request that finds the run already paused (a sibling branch paused it
//! first) joins that pause: its evidence is recorded, nothing transitions. A
//! cancelled run is not paused, and nothing is recorded.
//!
//! Each pause taken is recorded under [`SCRIPT_PAUSE_DIR`] with the attempts
//! it covers (`workflow_live_v2_script_host_pause_credit.rs`). A resume
//! replays the script, the covered attempts answer from their records, and
//! when it requests the recorded id again, the host answers
//! `{"resumed": true, "pause_id": <id>}` and the script continues past it --
//! only while the run executes, has been resumed since the pause, and every
//! covered attempt still stands. A pause is taken once per id, so a resumed
//! run never re-pauses on the evidence it was resumed past; a script that
//! needs to pause again asks under a new id.

use super::*;
use std::sync::atomic::{AtomicU8, Ordering};

/// Where the pauses a run took are recorded, relative to its run directory.
pub(super) const SCRIPT_PAUSE_DIR: &str = "v2/script-pauses";
/// The largest evidence object, serialized, one pause event carries whole.
const MAX_PAUSE_EVIDENCE_BYTES: usize = 64 * 1024;

use super::workflow_live_v2_script_host_pause_credit::{
    ScriptPauseRecord, covered_attempts, credit_holds,
};

/// Keep sibling pause requests in the order they enter the async host. The
/// filesystem transaction itself runs on Tokio's blocking pool, whose workers
/// may otherwise acquire the run lock in a different order.
fn pause_request_order_lock(run_dir: &std::path::Path) -> std::sync::Arc<tokio::sync::Mutex<()>> {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock, Weak};

    static LOCKS: OnceLock<Mutex<HashMap<std::path::PathBuf, Weak<tokio::sync::Mutex<()>>>>> =
        OnceLock::new();
    let mut locks = LOCKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .expect("script pause ordering registry poisoned");
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(run_dir).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(tokio::sync::Mutex::new(()));
    locks.insert(run_dir.to_path_buf(), Arc::downgrade(&lock));
    lock
}

enum PauseOutcome {
    Taken {
        joined: bool,
        event: Option<u64>,
    },
    /// Taken by an earlier execution, and this run was resumed past it.
    Passed,
    /// Taken by an earlier execution, but the run is paused again now.
    StillPaused,
    Cancelled,
}

/// What a pause request does to a run in `status`.
#[derive(Debug, PartialEq, Eq)]
enum PauseDisposition {
    /// The run executes: pause it.
    Pause,
    /// A sibling branch already paused it: record this pause with that one.
    Join,
    /// A cancel outranks a pause: take nothing.
    Cancelled,
    /// A run in this status executes no script.
    Refused,
}

fn pause_disposition(status: &archon_workflow::RunStatus) -> PauseDisposition {
    match status {
        archon_workflow::RunStatus::Planned | archon_workflow::RunStatus::Running => {
            PauseDisposition::Pause
        }
        archon_workflow::RunStatus::Paused => PauseDisposition::Join,
        archon_workflow::RunStatus::Cancelled => PauseDisposition::Cancelled,
        archon_workflow::RunStatus::NeedsReview
        | archon_workflow::RunStatus::Blocked
        | archon_workflow::RunStatus::Failed
        | archon_workflow::RunStatus::Completed => PauseDisposition::Refused,
    }
}

impl WorkflowScriptHost {
    /// Runs one `w.pause` host call. `Ok` only for a pause already taken.
    pub(super) async fn request_script_pause(
        &self,
        payload: &str,
    ) -> archon_workflow::WorkflowResult<String> {
        let request: ScriptHostRequest = serde_json::from_str(payload)?;
        let pause_id = request.id.trim().to_string();
        if pause_id.is_empty() {
            return Err(WorkflowError::SpecInvalid(
                "w.pause requires a non-empty string id".to_string(),
            ));
        }
        let store = self.runner.workflow_store.clone();
        let v2_store = self.runner.v2_store.clone();
        let run_id = self.runner.run_id.clone();
        let record_path = pause_record_path(&pause_id);
        let evidence = bounded_evidence(
            request
                .options
                .get("evidence")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
        );
        let resume = format!("archon workflow resume --live --yes {run_id}");
        let lock_store = store.clone();
        let lock_v2_store = v2_store.clone();
        let lock_run_id = run_id.clone();
        let lock_pause_id = pause_id.clone();
        let lock_resume = resume.clone();
        let lock_evidence = evidence.clone();
        let fixed_state_path = store
            .run_dir(&run_id)
            .join(crate::command::workflow_decompose_state::FIXED_STATE_PATH);
        let pause_order = pause_request_order_lock(&store.run_dir(&run_id))
            .lock_owned()
            .await;
        let persistence_stage = std::sync::Arc::new(AtomicU8::new(0));
        let worker_stage = persistence_stage.clone();
        let outcome = tokio::task::spawn_blocking(move || {
            // Keep the FIFO gate until the blocking transaction ends, even if
            // the async caller drops its future while this task is still writing.
            let _pause_order = pause_order;
            lock_store.with_run_lock(&lock_run_id, |locked| {
                let mut run = locked.load_state(&lock_run_id)?;
                // A stale session decides nothing: not a pause, a join or a pass.
                // Refused as every stale write is, before anything is read or
                // recorded: a restart since this session opened, or a resume that
                // gave the run to a newer executor.
                lock_v2_store.require_session_restart_epoch()?;
                lock_v2_store.require_session_executor(&run)?;
                // Issue 337: a sibling of a deliberate stop pauses nothing.
                let unreadable_stop = archon_workflow::control_pause::terminal_stop_before_pause(
                    locked,
                    &run,
                    &format!("pause '{lock_pause_id}'"),
                    false,
                )?;
                // Coverage and the grant share the lock with run control and
                // generation-owned persistence: no slot may change between them.
                let credit = Self::pause_credit(&lock_store, &lock_v2_store, &lock_run_id, &record_path, &lock_pause_id)?;
                // Run control first: a pause already taken passes only a run that
                // executes and was resumed since; a cancel outranks everything.
                let joined = match (pause_disposition(&run.status), &credit) {
                    (PauseDisposition::Cancelled, _) => return Ok(PauseOutcome::Cancelled),
                    (PauseDisposition::Refused, _) => {
                        return Err(WorkflowError::SpecInvalid(format!(
                            "w.pause('{lock_pause_id}') cannot pause run {lock_run_id} in status {:?}",
                            run.status
                        )));
                    }
                    (PauseDisposition::Pause, Some(taken)) if run.generation > taken.generation => {
                        return Ok(PauseOutcome::Passed);
                    }
                    (PauseDisposition::Join, Some(_)) => return Ok(PauseOutcome::StillPaused),
                    (PauseDisposition::Pause, _) => false,
                    (PauseDisposition::Join, None) => true,
                };
                // What the pause covers, read before anything transitions: the
                // attempts it lets a resume replay and the credit binds to.
                let covered = lock_v2_store
                    .load_call_records()
                    .map(|records| covered_attempts(&records))
                    .unwrap_or_else(|error| {
                        tracing::warn!(%error, pause_id = %lock_pause_id, "script pause covers no recorded attempt");
                        Vec::new()
                    });
                if !joined {
                    archon_workflow::control_pause::apply_pause(&mut run);
                    locked.save_state(&run)?;
                }
                worker_stage.store(1, Ordering::Release);
                // The run is paused from here whatever happens to the evidence.
                let detail = serde_json::json!({
                    "action": "pause",
                    "event": "script_pause",
                    "pause_id": lock_pause_id.clone(),
                    "joined": joined,
                    "generation": run.generation,
                    "evidence": lock_evidence.clone(),
                    "resume": lock_resume.clone(),
                    "terminal_stop_unreadable": unreadable_stop,
            });
            let kind = if joined {
                WorkflowEventKind::StageStalled
            } else {
                WorkflowEventKind::Paused
            };
            let event = emit_event(locked, &lock_run_id, kind, detail).map_err(|error| {
                WorkflowError::ControlPaused(format!(
                    "pause '{lock_pause_id}' was saved, but its evidence event was not recorded: {error}; run {lock_run_id} is paused, not failed; {lock_resume} resumes it"
                ))
            })?;
            worker_stage.store(2, Ordering::Release);
            let record = ScriptPauseRecord {
                pause_id: lock_pause_id.clone(),
                joined,
                event_seq: Some(event),
                generation: run.generation,
                covered,
                host_taken: false,
                judged_at_pause: Default::default(),
            };
            locked.write_run_json(&lock_run_id, &record_path, &record).map_err(|error| {
                WorkflowError::ControlPaused(format!(
                    "pause '{lock_pause_id}' was saved and its event recorded, but its replay record was not written: {error}; run {lock_run_id} is paused, not failed; {lock_resume} resumes it"
                ))
            })?;
            worker_stage.store(3, Ordering::Release);
            Ok(PauseOutcome::Taken {
                joined,
                event: Some(event),
            })
            })
        })
        .await
        .map_err(|error| {
            let evidence_gap = match persistence_stage.load(Ordering::Acquire) {
                0 => return WorkflowError::SpecInvalid(format!("pause transaction worker failed before saving the pause: {error}")),
                1 => "its evidence event may not have been recorded",
                2 => "its replay record may not have been written",
                _ => "its evidence and replay record were written",
            };
            WorkflowError::ControlPaused(format!(
                "pause '{pause_id}' was saved, but {evidence_gap}; transaction worker failed: {error}; run {run_id} is paused, not failed; {resume} resumes it"
            ))
        })??;
        let (joined, event) = match outcome {
            PauseOutcome::Cancelled => {
                return Err(WorkflowError::ControlCancelled(format!(
                    "run {run_id} is cancelled; pause '{pause_id}' was not taken"
                )));
            }
            PauseOutcome::StillPaused => {
                return Err(WorkflowError::ControlPaused(format!(
                    "pause '{pause_id}' was taken earlier, and run {run_id} is paused again; {resume} continues"
                )));
            }
            PauseOutcome::Passed => {
                return Ok(serde_json::json!({ "resumed": true, "pause_id": pause_id }).to_string());
            }
            PauseOutcome::Taken { joined, event } => (joined, event),
        };
        let cause = match (evidence["subject"].as_str(), evidence["reason"].as_str()) {
            (Some(subject), Some(reason)) => format!(" for {subject} ({reason})"),
            (Some(subject), None) => format!(" for {subject}"),
            _ => String::new(),
        };
        let message = format!(
            "script requested pause '{pause_id}'{cause}{}: the run is paused, not failed; {resume} continues past it",
            if joined {
                ", joining the pause in force"
            } else {
                ""
            }
        );
        tracing::warn!(run_id, "{message}");
        let fixed_store = store.clone();
        let fixed_run_id = run_id.clone();
        let fixed_pause_id = pause_id.clone();
        let fixed_evidence = evidence.clone();
        let failure_run_id = run_id.clone();
        tokio::task::spawn_blocking(move || {
            if fixed_state_path.exists() {
                let event = event.map_or_else(|| "none".to_string(), |seq| seq.to_string());
                append_fixed_log(
                    &fixed_store,
                    &fixed_run_id,
                    &format!(
                        "event_id={event} transition=script_pause pause_id={} joined={joined} subject={} reason={} next_action=resume run_id={fixed_run_id}",
                        crate::command::workflow_decompose_events::log_field(&fixed_pause_id),
                        crate::command::workflow_decompose_events::log_field(
                            fixed_evidence["subject"].as_str().unwrap_or("none")
                        ),
                        crate::command::workflow_decompose_events::log_field(
                            fixed_evidence["reason"].as_str().unwrap_or("none")
                        ),
                    ),
                );
            }
        })
        .await
        .map_err(|error| {
            WorkflowError::ControlPaused(format!(
                "pause '{pause_id}' was saved with its evidence, but its resume log was not written: {error}; run {failure_run_id} is paused, not failed; {resume} resumes it"
            ))
        })?;
        Err(WorkflowError::ControlPaused(message))
    }

    #[cfg(test)]
    pub(crate) async fn request_script_pause_for_test(
        &self,
        payload: &str,
    ) -> archon_workflow::WorkflowResult<String> {
        self.request_script_pause(payload).await
    }
}

impl WorkflowScriptHost {
    /// The record of `pause_id` taken by an earlier execution, while every
    /// attempt it covers still stands; `None` when there is none or a restart
    /// (or a later attempt) has voided it.
    fn pause_credit(
        store: &WorkflowStore,
        v2_store: &WorkflowV2ResultStore,
        run_id: &str,
        record_path: &str,
        pause_id: &str,
    ) -> archon_workflow::WorkflowResult<Option<ScriptPauseRecord>> {
        let path = store.run_dir(run_id).join(record_path);
        let raw = match std::fs::read(&path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(WorkflowError::Io { path, source }),
        };
        let record: ScriptPauseRecord = serde_json::from_slice(&raw)?;
        if record.pause_id != pause_id {
            return Ok(None);
        }
        let slots = v2_store.load_call_records()?;
        Ok(credit_holds(&record, &slots).then_some(record))
    }
}

/// The record path of `pause_id`: readable, and collision-free by digest.
pub(super) fn pause_record_path(pause_id: &str) -> String {
    let readable: String = pause_id
        .chars()
        .take(64)
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let digest = archon_workflow::task_set_contract::content_digest(pause_id.as_bytes());
    format!("{SCRIPT_PAUSE_DIR}/{readable}-{}.json", &digest[..16])
}

/// `evidence` whole when it is small enough, else its serialized text cut to
/// the bound: an operator still reads what the script said.
fn bounded_evidence(evidence: serde_json::Value) -> serde_json::Value {
    let text = evidence.to_string();
    if text.len() <= MAX_PAUSE_EVIDENCE_BYTES {
        return evidence;
    }
    let mut end = MAX_PAUSE_EVIDENCE_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    serde_json::json!({ "truncated": true, "bytes": text.len(), "text": &text[..end] })
}

fn emit_event(
    store: &WorkflowStore,
    run_id: &str,
    kind: WorkflowEventKind,
    detail: serde_json::Value,
) -> archon_workflow::WorkflowResult<u64> {
    let seq = store.next_event_seq(run_id)?;
    WorkflowEventLog::new(store.clone()).emit(run_id, seq, kind, detail)?;
    Ok(seq)
}

/// Appends `line` to a fixed decomposition's operator log. Evidence only: a
/// missing or unreadable log never changes the outcome.
fn append_fixed_log(store: &WorkflowStore, run_id: &str, line: &str) {
    let state_path = store
        .run_dir(run_id)
        .join(crate::command::workflow_decompose_state::FIXED_STATE_PATH);
    let appended = store.with_run_lock(run_id, |_| {
        std::fs::read(&state_path)
            .map_err(|source| WorkflowError::Io {
                path: state_path.clone(),
                source,
            })
            .and_then(|raw| {
                serde_json::from_slice::<archon_workflow::FixedDecompositionStateV1>(&raw)
                    .map_err(WorkflowError::from)
            })
            .and_then(|state| {
                crate::command::workflow_decompose_log::validated_fixed_log_path(
                    std::path::Path::new(&state.log_path),
                    &state.identity,
                )
            })
            .and_then(|path| {
                crate::command::workflow_decompose_log::append_nofollow_line(&path, line)
            })
    });
    if let Err(error) = appended {
        tracing::warn!(%error, "script pause log line not written");
    }
}

#[cfg(test)]
mod tests {
    use super::{PauseDisposition, pause_disposition, pause_record_path};
    use archon_workflow::RunStatus;

    #[test]
    fn a_pause_request_pauses_an_executing_run_joins_a_paused_one_and_yields_to_a_cancel() {
        for (status, expected) in [
            (RunStatus::Planned, PauseDisposition::Pause),
            (RunStatus::Running, PauseDisposition::Pause),
            (RunStatus::Paused, PauseDisposition::Join),
            (RunStatus::Cancelled, PauseDisposition::Cancelled),
            (RunStatus::NeedsReview, PauseDisposition::Refused),
            (RunStatus::Blocked, PauseDisposition::Refused),
            (RunStatus::Failed, PauseDisposition::Refused),
            (RunStatus::Completed, PauseDisposition::Refused),
        ] {
            assert_eq!(pause_disposition(&status), expected, "{status:?}");
        }
    }

    #[test]
    fn pause_ids_that_sanitize_alike_keep_distinct_records() {
        let slash = pause_record_path("pause-a/b-1");
        let colon = pause_record_path("pause-a:b-1");
        assert_ne!(slash, colon);
        assert!(
            slash.starts_with("v2/script-pauses/pause-a_b-1-"),
            "{slash}"
        );
        assert!(!slash.contains(".."), "{slash}");
        assert_eq!(pause_record_path("../../x").matches('/').count(), 2);
    }
}
