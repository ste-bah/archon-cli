//! Issue 296: an authoring loop that spends its bounded attempts without an
//! accepted script is a stall. It PAUSES the run with its evidence (the call,
//! the attempts, the last findings) and never fails it. A resume authors again
//! with a new agent, handed the last recorded rejection, and numbers its
//! attempts after the recorded ones, so no evidence is overwritten.
//!
//! Only a resume of an author stall is handed that rejection: the pause
//! writes [`STALL_MARKER`] and the next authoring takes it. A re-author for
//! any other reason (a deleted persisted script) starts from the brief alone.

use archon_workflow::{WorkflowError, WorkflowResult, WorkflowStore};

/// The call id of the authoring bootstrap's agent call (`V3_AUTHOR_BOOTSTRAP`).
pub(super) const AUTHOR_CALL_ID: &str = "author-workflow-script";

/// Under `rejected-scripts/`: the attempt an author stall paused on.
pub(super) const STALL_MARKER: &str = "stall-pause.json";

/// Why the authoring loop stopped without a script.
pub(super) enum AuthorStall {
    /// Every defect attempt was rejected by the dry-run pre-flight or came
    /// back without a usable script.
    Defects { reason: String },
    /// Every transport attempt died before a script came back.
    Transport { error: String },
}

/// The last recorded rejection, from which a resumed loop goes on.
#[derive(Debug, Default)]
pub(super) struct PriorRejections {
    /// The highest recorded attempt number (0 when none is recorded).
    pub(super) attempts: usize,
    /// Its reason and, when one was recorded, its rejected draft.
    pub(super) last: Option<(String, Option<String>)>,
}

/// Reads `rejected-scripts/attempt-N.json` records an earlier execution left.
/// Unreadable records are skipped: they are evidence, not state.
pub(super) fn prior_rejections(store: &WorkflowStore, run_id: &str) -> PriorRejections {
    let dir = store.run_dir(run_id).join("rejected-scripts");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return PriorRejections::default();
    };
    let highest = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            name.strip_prefix("attempt-")?
                .strip_suffix(".json")?
                .parse::<usize>()
                .ok()
        })
        .max()
        .unwrap_or(0);
    // Taken once: a later authoring for another reason starts afresh.
    let marker = dir.join(STALL_MARKER);
    let stalled_on = std::fs::read(&marker)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|value| value.get("attempt").and_then(serde_json::Value::as_u64));
    let _ = std::fs::remove_file(&marker);
    if highest == 0 || stalled_on != Some(highest as u64) {
        return PriorRejections {
            attempts: highest,
            last: None,
        };
    }
    let record: Option<serde_json::Value> =
        std::fs::read(dir.join(format!("attempt-{highest}.json")))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok());
    let last = record.and_then(|record| {
        let reason = record.get("error")?.as_str()?.to_string();
        let draft = record
            .get("script_path")
            .and_then(serde_json::Value::as_str)
            .and_then(|relative| {
                std::fs::read_to_string(store.run_dir(run_id).join(relative)).ok()
            });
        Some((reason, draft))
    });
    PriorRejections {
        attempts: highest,
        last,
    }
}

/// Pauses the run (owned by `generation`) on an authoring stall and returns
/// the pause as the error the run ends this execution with. When the pause
/// cannot be recorded (another generation owns the run), that error is
/// returned instead, carrying its reason.
pub(super) fn pause_on_author_stall(
    store: &WorkflowStore,
    run_id: &str,
    generation: u64,
    stall: AuthorStall,
    defect_attempts: usize,
    transport_attempts: usize,
    recorded_attempts: usize,
) -> WorkflowError {
    let resume = format!("archon workflow resume --live --yes {run_id}");
    let (cause, finding, headline) = match &stall {
        AuthorStall::Defects { reason } => (
            "defect_attempts_exhausted",
            reason.as_str(),
            format!(
                "authored workflow failed its dry-run pre-flight {defect_attempts} times; last error: {reason}"
            ),
        ),
        AuthorStall::Transport { error } => (
            "transport_attempts_exhausted",
            error.as_str(),
            format!(
                "the workflow author call failed in transport {transport_attempts} times; last error: {error}"
            ),
        ),
    };
    let last_rejection = (recorded_attempts > 0)
        .then(|| format!("rejected-scripts/attempt-{recorded_attempts}.json"));
    let detail = serde_json::json!({
        "event": "author_stall_pause",
        "cause": "no_progress",
        "stall": cause,
        "call_id": AUTHOR_CALL_ID,
        "defect_attempts": defect_attempts,
        "transport_attempts": transport_attempts,
        "recorded_rejections": recorded_attempts,
        "last_finding": finding,
        "last_rejection": last_rejection,
        "resume": resume,
    });
    // The marker goes first: a resume of a recorded pause must find it. One
    // that is not written costs only the hand-over of the last finding.
    if recorded_attempts > 0 {
        let marker = serde_json::json!({ "attempt": recorded_attempts });
        let path = format!("rejected-scripts/{STALL_MARKER}");
        if let Err(error) = store.write_run_json(run_id, path, &marker) {
            tracing::warn!(%error, run_id, "author stall marker not written");
        }
    }
    match archon_workflow::control_pause::pause_with_evidence(store, run_id, generation, detail) {
        Ok(event) => {
            if let Err(error) = event {
                tracing::warn!(%error, run_id, "author stall pause event not recorded");
            }
            WorkflowError::ControlPaused(format!(
                "{headline}; run {run_id} is paused, not failed. The rejected drafts and their findings are under rejected-scripts/; fix what they name if needed, then {resume}: authoring goes on with a new agent from the last finding"
            ))
        }
        Err(error) => error,
    }
}

/// Run control (an operator pause or cancel) stops the authoring loop as it
/// is; it is never an authoring attempt.
pub(super) fn is_run_control(error: &WorkflowError) -> bool {
    matches!(
        error,
        WorkflowError::ControlPaused(_) | WorkflowError::ControlCancelled(_)
    )
}

/// The run's generation when authoring starts: the one a stall pause names.
pub(super) fn authoring_generation(store: &WorkflowStore, run_id: &str) -> WorkflowResult<u64> {
    Ok(store.load_state(run_id)?.generation)
}
