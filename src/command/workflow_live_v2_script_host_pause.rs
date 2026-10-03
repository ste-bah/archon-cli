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
//! `workflow pause` makes (status, running stages and items, generation),
//! with one evidence event carrying the script's evidence. The call then ends
//! with the pause error, like any call a pause stops, and the script unwinds.
//! A request that finds the run already paused (a sibling branch paused it
//! first) joins that pause: its evidence is recorded, nothing transitions. A
//! cancelled run is not paused, and nothing is recorded.
//!
//! Each pause taken is recorded under [`SCRIPT_PAUSE_DIR`]. A resume replays
//! the script; when it requests a recorded id again, the host answers
//! `{"resumed": true, "pause_id": <id>}` and the script continues past it. A
//! pause is taken once per id, so a resumed run never re-pauses on the
//! evidence it was resumed past; a script that needs to pause again asks
//! under a new id.

use super::*;

/// Where the pauses a run took are recorded, relative to its run directory.
pub(super) const SCRIPT_PAUSE_DIR: &str = "v2/script-pauses";
/// The largest evidence object, serialized, one pause event carries whole.
const MAX_PAUSE_EVIDENCE_BYTES: usize = 64 * 1024;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct ScriptPauseRecord {
    pause_id: String,
    joined: bool,
    event_seq: Option<u64>,
}

enum PauseOutcome {
    Taken { joined: bool, event: Option<u64> },
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
        let store = &self.runner.workflow_store;
        let run_id = self.runner.run_id.as_str();
        let record_path = pause_record_path(&pause_id);
        if pause_taken(store, run_id, &record_path, &pause_id)? {
            return Ok(serde_json::json!({ "resumed": true, "pause_id": pause_id }).to_string());
        }
        let evidence = bounded_evidence(
            request
                .options
                .get("evidence")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
        );
        let resume = format!("archon workflow resume --live --yes {run_id}");
        let outcome = store.with_run_lock(run_id, |locked| {
            let mut run = locked.load_state(run_id)?;
            let joined = match pause_disposition(&run.status) {
                PauseDisposition::Pause => false,
                PauseDisposition::Join => true,
                PauseDisposition::Cancelled => return Ok(PauseOutcome::Cancelled),
                PauseDisposition::Refused => {
                    return Err(WorkflowError::SpecInvalid(format!(
                        "w.pause('{pause_id}') cannot pause run {run_id} in status {:?}",
                        run.status
                    )));
                }
            };
            if !joined {
                pause_run_state(&mut run);
                locked.save_state(&run)?;
            }
            // The run is paused from here whatever happens to the evidence.
            let detail = serde_json::json!({
                "action": "pause",
                "event": "script_pause",
                "pause_id": pause_id,
                "joined": joined,
                "generation": run.generation,
                "evidence": evidence,
                "resume": resume,
            });
            let kind = if joined {
                WorkflowEventKind::StageStalled
            } else {
                WorkflowEventKind::Paused
            };
            let event = emit_event(locked, run_id, kind, detail)
                .inspect_err(|error| tracing::warn!(%error, "script pause event not recorded"))
                .ok();
            let record = ScriptPauseRecord {
                pause_id: pause_id.clone(),
                joined,
                event_seq: event,
            };
            if let Err(error) = locked.write_run_json(run_id, &record_path, &record) {
                // Not recorded means a resume asks again and pauses once more:
                // a second stop, never a lost one.
                tracing::warn!(%error, pause_id, "script pause record not written");
            }
            Ok(PauseOutcome::Taken { joined, event })
        })?;
        let (joined, event) = match outcome {
            PauseOutcome::Cancelled => {
                return Err(WorkflowError::ControlCancelled(format!(
                    "run {run_id} is cancelled; pause '{pause_id}' was not taken"
                )));
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
        if self.fixed_decomposition_state_present() {
            let event = event.map_or_else(|| "none".to_string(), |seq| seq.to_string());
            append_fixed_log(
                store,
                run_id,
                &format!(
                    "event_id={event} transition=script_pause pause_id={} joined={joined} subject={} reason={} next_action=resume run_id={run_id}",
                    crate::command::workflow_decompose_events::log_field(&pause_id),
                    crate::command::workflow_decompose_events::log_field(
                        evidence["subject"].as_str().unwrap_or("none")
                    ),
                    crate::command::workflow_decompose_events::log_field(
                        evidence["reason"].as_str().unwrap_or("none")
                    ),
                ),
            );
        }
        Err(WorkflowError::ControlPaused(message))
    }
}

/// The transition `workflow pause` makes, applied to a run this host owns.
fn pause_run_state(run: &mut archon_workflow::WorkflowRun) {
    run.status = archon_workflow::RunStatus::Paused;
    for stage in run.stages.values_mut() {
        if stage.status == archon_workflow::StageStatus::Running {
            stage.status = archon_workflow::StageStatus::Paused;
            stage.completed_at = None;
        }
    }
    for item in run.items.values_mut() {
        if item.status == archon_workflow::StageStatus::Running {
            item.status = archon_workflow::StageStatus::Paused;
        }
    }
    run.generation = run.generation.saturating_add(1);
    run.mark_updated();
}

/// The record path of `pause_id`: readable, and collision-free by digest.
fn pause_record_path(pause_id: &str) -> String {
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

/// Whether `pause_id` was taken by an earlier execution of this run.
fn pause_taken(
    store: &WorkflowStore,
    run_id: &str,
    record_path: &str,
    pause_id: &str,
) -> archon_workflow::WorkflowResult<bool> {
    let path = store.run_dir(run_id).join(record_path);
    let raw = match std::fs::read(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(source) => {
            return Err(WorkflowError::Io {
                path: path.clone(),
                source,
            });
        }
    };
    let record: ScriptPauseRecord = serde_json::from_slice(&raw)?;
    Ok(record.pause_id == pause_id)
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
    let appended = std::fs::read(&state_path)
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
        .and_then(|path| crate::command::workflow_decompose_log::append_nofollow_line(&path, line));
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
