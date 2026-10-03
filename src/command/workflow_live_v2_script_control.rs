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
//   test, and the host can read back from the rejection without its text;
// - the run's outcome follows the stored run state first: Paused or Cancelled
//   outranks every result or error the script produced while it unwound.
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

    fn parse(kind: &str) -> Option<Self> {
        match kind {
            "pause" => Some(Self::Pause),
            "cancel" => Some(Self::Cancel),
            _ => None,
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

/// What the script receives for a host call that failed with `err`: a control
/// envelope when run control stopped the call, `None` for every other error.
/// The stored run state names the kind, so a call whose persistence lost
/// generation ownership to a pause (raised as a cancel) still says "pause".
pub(super) fn control_envelope(
    store: &WorkflowStore,
    run_id: &str,
    err: &WorkflowError,
) -> Option<String> {
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
    let mut envelope = serde_json::Map::new();
    envelope.insert(
        WORKFLOW_CONTROL_ENVELOPE_KEY.to_string(),
        serde_json::json!({
            "code": WORKFLOW_CONTROL_CODE,
            "kind": kind.as_str(),
            "message": message,
        }),
    );
    Some(serde_json::Value::Object(envelope).to_string())
}

/// A script promise that rejected: its text, and the run-control kind when
/// the rejection is a `WorkflowControlError`.
pub(super) struct ScriptRejection {
    pub(super) message: String,
    pub(super) control: Option<RunControlKind>,
}

impl ScriptRejection {
    pub(super) fn caught(err: &rquickjs::CaughtError<'_>) -> Self {
        Self {
            message: rquickjs::Error::new_from_js_message("workflow.js", "string", err.to_string())
                .to_string(),
            control: caught_control_kind(err),
        }
    }
}

/// The typed kind a rejection carries: read from the thrown object's `code`
/// and `kind`, never from its message.
fn caught_control_kind(err: &rquickjs::CaughtError<'_>) -> Option<RunControlKind> {
    let object = match err {
        rquickjs::CaughtError::Exception(exception) => exception.as_object().clone(),
        rquickjs::CaughtError::Value(value) => value.as_object()?.clone(),
        rquickjs::CaughtError::Error(_) => return None,
    };
    let code: Option<String> = object.get("code").ok()?;
    if code.as_deref() != Some(WORKFLOW_CONTROL_CODE) {
        return None;
    }
    let kind: String = object.get("kind").ok()?;
    RunControlKind::parse(&kind)
}

/// The run's outcome when run control decides it, by fixed priority:
/// 1. the stored run state, Paused or Cancelled, over anything the script
///    returned or threw;
/// 2. a typed `WorkflowControlError` the script rejected with.
///
/// `None` leaves the script's own outcome in force.
pub(super) fn control_outcome(
    store: &WorkflowStore,
    run_id: &str,
    rejection: Option<&ScriptRejection>,
) -> Option<WorkflowError> {
    if let Some(kind) = stored_run_control(store, run_id) {
        let message = match rejection {
            Some(rejection) if rejection.control == Some(kind) => {
                from_marker(&rejection.message, kind)
            }
            Some(rejection) => format!(
                "run {run_id} is {} by run control, which outranks the script's own error: {}",
                kind.state_word(),
                rejection.message
            ),
            None => format!(
                "run {run_id} is {} by run control, which outranks the script's own result",
                kind.state_word()
            ),
        };
        return Some(kind.into_error(message));
    }
    let rejection = rejection?;
    let kind = rejection.control?;
    Some(kind.into_error(from_marker(&rejection.message, kind)))
}

/// The message from the kind's marker on, as `workflow_js_error` cut it.
fn from_marker(message: &str, kind: RunControlKind) -> String {
    message.find(kind.marker()).map_or_else(
        || message.to_string(),
        |start| message[start..].trim().to_string(),
    )
}
