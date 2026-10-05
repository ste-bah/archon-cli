//! Issue 296: an authoring loop that spends its bounded attempts without an
//! accepted script is a stall. It PAUSES the run with its evidence (the call,
//! the attempts, the last findings) and never fails it. A resume authors again
//! with a new agent, handed the last recorded rejection, and numbers its
//! attempts after the recorded ones, so no evidence is overwritten.
//!
//! Only a resume of an author stall is handed that rejection: the pause
//! writes [`STALL_MARKER`] and the next authoring takes it. A re-author for
//! any other reason (a deleted persisted script) starts from the brief alone.
//!
//! Issue 324: the marker names only the rejection the stalled authoring
//! itself held (one it recorded, or one a stall handed it), never an older
//! one; it is removed only once the next authoring's first attempt has a
//! durable result, so a resume stopped before that keeps the last finding.
//! An I/O fault or a damaged store during authoring is not a defect the
//! author can fix: it pauses the run at once with the fault as evidence.

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
    /// An I/O fault or a damaged store (Issue 324): nothing an author can fix,
    /// and nothing a retry in this execution would change.
    Infrastructure { error: String },
}

/// The attempts an authoring stall reports.
pub(super) struct StallAttempts {
    pub(super) defects: usize,
    pub(super) transports: usize,
    /// The highest recorded rejection number.
    pub(super) recorded: usize,
    /// The recorded rejection the stalled authoring held as its finding, if
    /// any: the only one the pause names and hands to a resume.
    pub(super) held: Option<usize>,
}

/// An I/O fault or a damaged store, as opposed to an authoring defect.
pub(super) fn is_infrastructure_fault(error: &WorkflowError) -> bool {
    matches!(
        error,
        WorkflowError::Io { .. } | WorkflowError::StateCorrupt(_)
    )
}

/// Removes the hand-over marker once the authoring it was taken by has a
/// durable first result (a recorded rejection or the persisted script).
pub(super) fn clear_marker(store: &WorkflowStore, run_id: &str) {
    let marker = store
        .run_dir(run_id)
        .join("rejected-scripts")
        .join(STALL_MARKER);
    match std::fs::remove_file(&marker) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            tracing::warn!(%error, run_id, "author stall marker not removed");
        }
        _ => {}
    }
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
    // Read here, removed by [`clear_marker`] once this authoring's first
    // attempt has a durable result: a later authoring for another reason
    // starts afresh, a resume stopped before then keeps the finding.
    let stalled_on = std::fs::read(dir.join(STALL_MARKER))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|value| value.get("attempt").and_then(serde_json::Value::as_u64));
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
    attempts: StallAttempts,
) -> WorkflowError {
    let StallAttempts {
        defects: defect_attempts,
        transports: transport_attempts,
        recorded: recorded_attempts,
        held,
    } = attempts;
    let resume = format!("archon workflow resume --live --yes {run_id}");
    let (stalled, finding, headline) = match &stall {
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
        AuthorStall::Infrastructure { error } => (
            "infrastructure_fault",
            error.as_str(),
            format!(
                "authoring stopped on an infrastructure fault (I/O or a damaged run store), not an authoring defect: {error}"
            ),
        ),
    };
    // A fault is not a stall of the author: it is named apart from one.
    let cause = match &stall {
        AuthorStall::Infrastructure { .. } => "infrastructure_fault",
        _ => "no_progress",
    };
    let next = match &stall {
        AuthorStall::Infrastructure { .. } => {
            "restore what the fault names (the rejected drafts and their findings stay under rejected-scripts/)"
        }
        _ => {
            "the rejected drafts and their findings are under rejected-scripts/; fix what they name if needed"
        }
    };
    let held = held.filter(|attempt| *attempt > 0);
    let last_rejection = held.map(|attempt| format!("rejected-scripts/attempt-{attempt}.json"));
    let detail = serde_json::json!({
        "event": "author_stall_pause",
        "cause": cause,
        "stall": stalled,
        "call_id": AUTHOR_CALL_ID,
        "defect_attempts": defect_attempts,
        "transport_attempts": transport_attempts,
        "recorded_rejections": recorded_attempts,
        "last_finding": finding,
        "last_rejection": last_rejection,
        "resume": resume,
    });
    // The marker goes first: a resume of a recorded pause must find it. One
    // that is not written costs only the hand-over of the last finding. With
    // no held rejection, an older marker would hand over a stale one. Issue
    // 324: only while `generation` owns the run (#291), checked under the run
    // lock first: a stale executor changes no marker and pauses nothing.
    let owned = store.with_run_lock(run_id, |locked| {
        archon_workflow::control_pause::require_generation(locked, run_id, generation)?;
        match held {
            Some(attempt) => {
                let marker = serde_json::json!({ "attempt": attempt });
                let path = format!("rejected-scripts/{STALL_MARKER}");
                if let Err(error) = locked.write_run_json(run_id, path, &marker) {
                    tracing::warn!(%error, run_id, "author stall marker not written");
                }
            }
            None => clear_marker(locked, run_id),
        }
        Ok(())
    });
    if let Err(refused) = owned {
        return refused;
    }
    match archon_workflow::control_pause::pause_with_evidence(store, run_id, generation, detail) {
        Ok(event) => {
            if let Err(error) = event {
                tracing::warn!(%error, run_id, "author stall pause event not recorded");
            }
            WorkflowError::ControlPaused(format!(
                "{headline}; run {run_id} is paused, not failed. Next: {next}, then {resume}: authoring goes on with a new agent from the last finding"
            ))
        }
        Err(error) => error,
    }
}

/// Pauses on an infrastructure fault met outside the authoring loop (reading
/// the persisted script), naming the recorded rejections and handing none
/// over. A store whose state cannot be read cannot be paused: that error is
/// returned as it is.
pub(super) fn pause_on_infrastructure_fault(
    store: &WorkflowStore,
    run_id: &str,
    error: String,
) -> WorkflowError {
    let generation = match authoring_generation(store, run_id) {
        Ok(generation) => generation,
        Err(unreadable) => return unreadable,
    };
    let attempts = StallAttempts {
        defects: 0,
        transports: 0,
        recorded: prior_rejections(store, run_id).attempts,
        held: None,
    };
    let stall = AuthorStall::Infrastructure { error };
    pause_on_author_stall(store, run_id, generation, stall, attempts)
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

/// Round 2 (b): a stale generation changes no marker and pauses nothing.
#[cfg(test)]
#[path = "workflow_live_v3_author_stall_owner_tests.rs"]
mod owner_tests;
