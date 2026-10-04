// Issue-253: run control (pause, cancel) as a typed outcome.
//
// A pause or cancel used to reach the script as a plain thrown error, the
// same shape as a call that ran out of attempts, and the run's outcome was
// read back from the text of whatever the script threw last. A pool that
// raises its lowest-index failure therefore reported an exhausted sibling,
// or a sibling's "cancelled" error that the pause itself induced, instead of
// the pause. Two rules replace that:
// - a host call that run control stopped resolves to a control envelope, and
//   the prelude turns it into a `WorkflowControlError` (`code` is
//   `WORKFLOW_CONTROL_CODE`, `kind` is "pause" or "cancel") the script can
//   test;
// - a newer stored stop or a typed stop observed by the host outranks every
//   script result and error. Host evidence survives a resume during unwinding.
//   Scripts cannot create that evidence by throwing objects or matching text.
use super::*;

use archon_workflow::RunStatus;
use archon_workflow::v2::script::{WORKFLOW_CONTROL_CODE, WORKFLOW_CONTROL_ENVELOPE_KEY};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RunControlKind {
    Pause,
    Cancel,
}

impl RunControlKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pause => "pause",
            Self::Cancel => "cancel",
        }
    }

    fn of_error(err: &WorkflowError) -> Option<Self> {
        match err {
            WorkflowError::ControlPaused(_) => Some(Self::Pause),
            WorkflowError::ControlCancelled(_) => Some(Self::Cancel),
            _ => None,
        }
    }

    fn of_status(status: &RunStatus) -> Option<Self> {
        match status {
            RunStatus::Paused => Some(Self::Pause),
            RunStatus::Cancelled => Some(Self::Cancel),
            _ => None,
        }
    }

    /// The prefix `WorkflowError`'s Display gives this kind.
    fn marker(self) -> &'static str {
        match self {
            Self::Pause => "workflow paused by run control:",
            Self::Cancel => "workflow cancelled by run control:",
        }
    }

    fn state_word(self) -> &'static str {
        match self {
            Self::Pause => "paused",
            Self::Cancel => "cancelled",
        }
    }

    fn into_error(self, message: String) -> WorkflowError {
        match self {
            Self::Pause => WorkflowError::ControlPaused(message),
            Self::Cancel => WorkflowError::ControlCancelled(message),
        }
    }
}

/// The run control the stored run state records, if any. An unreadable state
/// records none: the caller then keeps the outcome it already had.
fn stored_run_control(store: &WorkflowStore, run_id: &str) -> Option<RunControlKind> {
    RunControlKind::of_status(&store.load_state(run_id).ok()?.status)
}

/// Trusted evidence from this executor's host boundary, never from JavaScript.
#[derive(Clone, Debug)]
pub(super) struct HostControlStop {
    kind: RunControlKind,
    message: String,
}

/// What the script receives for a host call that failed with `err`: a control
/// envelope when run control stopped the call, `None` for every other error.
/// The stored run state names the kind, so a call whose persistence lost
/// generation ownership to a pause (raised as a cancel) still says "pause".
pub(super) fn control_envelope(
    store: &WorkflowStore,
    run_id: &str,
    err: &WorkflowError,
) -> Option<(String, HostControlStop)> {
    let raised = RunControlKind::of_error(err)?;
    let kind = stored_run_control(store, run_id).unwrap_or(raised);
    let message = if kind == raised {
        err.to_string()
    } else {
        format!(
            "{} run {run_id} is {}; this call stopped with: {err}",
            kind.marker(),
            kind.state_word()
        )
    };
    let observed = HostControlStop {
        kind,
        message: message.clone(),
    };
    let mut envelope = serde_json::Map::new();
    envelope.insert(
        WORKFLOW_CONTROL_ENVELOPE_KEY.to_string(),
        serde_json::json!({
            "code": WORKFLOW_CONTROL_CODE,
            "kind": kind.as_str(),
            "message": message,
        }),
    );
    Some((serde_json::Value::Object(envelope).to_string(), observed))
}

/// The stored run generation this executor started under.
#[derive(Clone, Copy, Debug)]
pub(super) struct ExecutorStart {
    generation: Option<u64>,
}

impl ExecutorStart {
    /// The generation, `None` when the state was unreadable at start.
    pub(super) fn generation(&self) -> Option<u64> {
        self.generation
    }
}

/// Reads the stored run state before the script runs. A run already paused
/// or cancelled is reported so at once: the script does not run, and no later
/// failure of it can stand in for that control decision. An unreadable state
/// correlates nothing (`generation: None`).
pub(super) fn observe_start(
    store: &WorkflowStore,
    run_id: &str,
) -> Result<ExecutorStart, WorkflowError> {
    let Ok(run) = store.load_state(run_id) else {
        return Ok(ExecutorStart { generation: None });
    };
    if let Some(kind) = RunControlKind::of_status(&run.status) {
        return Err(kind.into_error(format!(
            "run {run_id} was already {} (generation {}) when this executor started; the script did not run",
            kind.state_word(),
            run.generation
        )));
    }
    Ok(ExecutorStart {
        generation: Some(run.generation),
    })
}

/// A newer stored stop takes precedence. Otherwise a host-observed stop
/// still abandons this executor, even if another executor resumed the run.
/// Script-authored rejection text never decides the kind.
pub(super) fn control_outcome(
    store: &WorkflowStore,
    run_id: &str,
    start: ExecutorStart,
    observed: Option<&HostControlStop>,
    rejection: Option<&str>,
) -> Option<WorkflowError> {
    let stored = store.load_state(run_id).ok().and_then(|run| {
        if start
            .generation
            .is_some_and(|generation| run.generation <= generation)
        {
            None
        } else {
            RunControlKind::of_status(&run.status)
        }
    });
    let kind = stored.or_else(|| observed.map(|stop| stop.kind))?;
    let message = match rejection {
        // Its text only words the outcome; trusted host evidence decided it.
        Some(message) if message.contains(kind.marker()) => from_marker(message, kind),
        Some(message) => format!(
            "run {run_id} is {} by run control, which outranks the script's own error: {message}",
            kind.state_word()
        ),
        None if observed.is_some_and(|stop| stop.kind == kind) => observed?.message.clone(),
        None => format!(
            "run {run_id} is {} by run control, which outranks the script's own result",
            kind.state_word()
        ),
    };
    Some(kind.into_error(message))
}

/// The message from the kind's marker on.
fn from_marker(message: &str, kind: RunControlKind) -> String {
    message.find(kind.marker()).map_or_else(
        || message.to_string(),
        |start| message[start..].trim().to_string(),
    )
}
