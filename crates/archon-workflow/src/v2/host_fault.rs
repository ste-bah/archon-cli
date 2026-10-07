//! A stage that never ran, and what a run is allowed to do about it.
//!
//! A host call can fail two ways that look identical to a generated script:
//! the work ran and did not satisfy its verifier, or the host could not read
//! or write its own run state and so never dispatched anything. The second is
//! not an attempt by the task. Charging it as one spends a task's remediation
//! budget on something that says nothing about the work, and — because such a
//! failure returns in microseconds — a bounded remediation loop can spend the
//! whole budget before a second has passed, then do the same for the next
//! task, and the next. That is how one unreadable file in the run store turned
//! into a destroyed run rather than a single failed stage.
//!
//! Two guards live here, both keyed on the typed error rather than on timing.
//! Equal start and finish timestamps are NOT usable as the signal: every call
//! record stamps both from one clock read, so they are equal for a stage that
//! ran for an hour too.

use crate::WorkflowError;
use crate::v2::review_findings::HOST_REVIEW_FINDINGS_KEY;
use crate::v2::script::failed_v2_result;
use crate::v2::{WorkflowV2EvidenceKind, WorkflowV2Result};

/// The marker this crate stamps on a result for a call that never executed.
pub const HOST_FAULT_NO_VERDICT_MARKER: &str = "host_fault_no_verdict";

/// The marker the frozen script primitives already key their attempt refund
/// on. Their pool is documented as "an attempt burned by something that says
/// nothing about the work", which is exactly what a call that never ran is, so
/// a never-ran result joins that pool instead of inventing a second one the
/// primitives cannot see.
pub const NO_VERDICT_REFUND_MARKER: &str = "transport_failure_no_verdict";

/// Issue 337: the marker on EVERY result the host made from a dispatch error
/// (a provider, transport or host fault): the call returned no answer of its
/// own. Such a record is history, never a verdict a resume may replay.
pub const HOST_DISPATCH_ERROR_MARKER: &str = "host_dispatch_error";

/// The marker on a result this build made from a call's own answer that
/// failed validation: the model answered, wrongly, so it IS a verdict. It
/// keeps such a result apart from the unmarked legacy dispatch-error shape.
pub const INVALID_ANSWER_MARKER: &str = "invalid_answer";

/// Does the result recorded for call `call_id` carry no verdict on the work:
/// a dispatch error, a never-ran fault, or a refunded no-verdict attempt --
/// marked, or in the unmarked shape an older binary wrote for a dispatch
/// error?
pub fn result_carries_no_verdict(call_id: &str, result: &WorkflowV2Result) -> bool {
    [
        HOST_DISPATCH_ERROR_MARKER,
        HOST_FAULT_NO_VERDICT_MARKER,
        NO_VERDICT_REFUND_MARKER,
    ]
    .iter()
    .any(|marker| {
        result
            .data
            .get(*marker)
            .and_then(serde_json::Value::as_bool)
            == Some(true)
    }) || legacy_dispatch_error(call_id, result)
}

/// A dispatch error as a binary before [`HOST_DISPATCH_ERROR_MARKER`] wrote
/// it: the [`failed_v2_result`] of this call for its own recorded error,
/// rebuilt from the call id and compared on every field, never matched on
/// its text. A run started on such a binary may resume on this one, so its
/// fault records must not replay as verdicts. Every result this build makes
/// from `failed_v2_result` carries a marker of its kind (a dispatch error, or
/// an invalid answer), and a marker is no key the rule allows, so the rule
/// never reads a result of this build.
///
/// The old binary (8b7a3c13f) changed the result after it built it, in
/// `workflow_live_v2_script_host_exec.rs:403-413`: the normalizers of
/// `helpers_a.rs:400-428` attach the host review findings to `data` under
/// [`HOST_REVIEW_FINDINGS_KEY`] (`review_findings.rs:338`) for a call with a
/// review contract, and `mark_unresolved_dependency_metadata`
/// (`helpers_a.rs:430-466`) appends one review evidence and one review gap
/// for a dynamic wave. Its other normalizers change only accepted results,
/// artifacts, task coverage or `data.items`, none of which a failed result
/// has. So the rule allows exactly those additions: that key in `data`, and
/// evidence of kind review and gaps of severity review after the built ones.
fn legacy_dispatch_error(call_id: &str, result: &WorkflowV2Result) -> bool {
    let Some(data) = result.data.as_object() else {
        return false;
    };
    if data
        .keys()
        .any(|key| key != "error" && key != HOST_REVIEW_FINDINGS_KEY)
    {
        return false;
    }
    let Some(error) = data.get("error").and_then(serde_json::Value::as_str) else {
        return false;
    };
    let built = failed_v2_result(call_id, error);
    let mut stable = result.clone();
    let added_evidence = stable
        .evidence
        .split_off(built.evidence.len().min(stable.evidence.len()));
    let added_gaps = stable
        .residual_gaps
        .split_off(built.residual_gaps.len().min(stable.residual_gaps.len()));
    if let Some(data) = stable.data.as_object_mut() {
        data.remove(HOST_REVIEW_FINDINGS_KEY);
    }
    added_evidence
        .iter()
        .all(|evidence| evidence.kind == WorkflowV2EvidenceKind::Review)
        && added_gaps
            .iter()
            .all(|gap| gap.severity.as_deref() == Some("review"))
        && stable == built
}

/// The result for a call whose own answer failed validation: failed, and
/// marked as the answer it is ([`INVALID_ANSWER_MARKER`]).
pub fn invalid_answer_result(call_id: &str, error: WorkflowError) -> WorkflowV2Result {
    let mut result = failed_v2_result(call_id, error);
    if let Some(object) = result.data.as_object_mut() {
        object.insert(
            INVALID_ANSWER_MARKER.to_string(),
            serde_json::Value::Bool(true),
        );
    }
    result
}

/// Did this error end the call before anything could produce a verdict?
///
/// True only for the host failing on its own state. Everything a dispatched
/// call can fail with — a blocked or failed stage, a port error from the
/// provider, the host's own dispatch timer — is an attempt that happened and
/// is charged as one.
pub fn is_never_started_fault(error: &WorkflowError) -> bool {
    matches!(error, WorkflowError::StateCorrupt(_))
}

/// The typed result for a host call that raised `error`.
///
/// Identical to [`failed_v2_result`] except that a call which never started
/// carries the never-ran markers, so a budget that funds attempts is not
/// charged for one that did not happen.
pub fn v2_result_for_call_error(call_id: &str, error: &WorkflowError) -> WorkflowV2Result {
    let mut result = failed_v2_result(call_id, error);
    if let Some(object) = result.data.as_object_mut() {
        object.insert(
            HOST_DISPATCH_ERROR_MARKER.to_string(),
            serde_json::Value::Bool(true),
        );
    }
    // Batch G2: the host's own operational error produced no verdict on the
    // work: refunded like a dropped transport and typed `execution`, so the
    // script retries it without spending a round and no task is charged.
    if matches!(error, WorkflowError::HostOperational(_))
        && let Some(object) = result.data.as_object_mut()
    {
        object.insert(
            NO_VERDICT_REFUND_MARKER.to_string(),
            serde_json::Value::Bool(true),
        );
        object.insert("failure_kind".to_string(), serde_json::json!("execution"));
    }
    if is_never_started_fault(error)
        && let Some(object) = result.data.as_object_mut()
    {
        object.insert(
            HOST_FAULT_NO_VERDICT_MARKER.to_string(),
            serde_json::Value::Bool(true),
        );
        object.insert(
            NO_VERDICT_REFUND_MARKER.to_string(),
            serde_json::Value::Bool(true),
        );
    }
    result
}

/// Issue 324: is this the host failing on its own state — an I/O fault or a
/// damaged store — rather than a verdict, a provider error or transport?
/// Such a fault keeps its kind through dispatch, so a caller that must not
/// count it against the work (the v3 authoring loop) can tell it apart.
pub fn is_host_infrastructure_fault(error: &WorkflowError) -> bool {
    matches!(
        error,
        WorkflowError::Io { .. } | WorkflowError::StateCorrupt(_)
    )
}

/// A copy of an infrastructure fault (`WorkflowError` is not `Clone`), for a
/// caller that must both report it and keep it typed; `None` for any other.
pub fn copy_host_infrastructure_fault(error: &WorkflowError) -> Option<WorkflowError> {
    match error {
        WorkflowError::Io { path, source } => Some(WorkflowError::Io {
            path: path.clone(),
            source: std::io::Error::new(source.kind(), source.to_string()),
        }),
        WorkflowError::StateCorrupt(message) => Some(WorkflowError::StateCorrupt(message.clone())),
        _ => None,
    }
}

impl crate::v2::WorkflowV2AgentError {
    /// Host control and infrastructure faults keep their kind through the
    /// agent layer. Only ordinary provider failures become transport.
    pub fn from_call_error(error: &WorkflowError) -> Self {
        match error {
            WorkflowError::ControlPaused(message) => Self::ControlPaused(message.clone()),
            WorkflowError::ControlCancelled(message) => Self::ControlCancelled(message.clone()),
            WorkflowError::Io { path, source } => Self::HostIo {
                path: path.clone(),
                kind: source.kind(),
                message: source.to_string(),
            },
            WorkflowError::StateCorrupt(message) => Self::HostStateCorrupt(message.clone()),
            other => Self::Transport(other.to_string()),
        }
    }

    pub fn is_host_fault(&self) -> bool {
        matches!(self, Self::HostIo { .. } | Self::HostStateCorrupt(_))
    }

    /// Recover typed host control and infrastructure faults at the run layer.
    /// An ordinary agent failure remains a failed stage.
    pub fn into_workflow_error(self) -> WorkflowError {
        match self {
            Self::ControlPaused(message) => WorkflowError::ControlPaused(message),
            Self::ControlCancelled(message) => WorkflowError::ControlCancelled(message),
            Self::HostIo {
                path,
                kind,
                message,
            } => WorkflowError::Io {
                path,
                source: std::io::Error::new(kind, message),
            },
            Self::HostStateCorrupt(message) => WorkflowError::StateCorrupt(message),
            other => WorkflowError::StageFailed(other.to_string()),
        }
    }
}

/// Does this recorded result belong to a call that never started?
///
/// Read back from the typed marker rather than re-derived, so the run-scoped
/// streak below and the result a script was handed can never disagree.
pub fn result_reports_never_started(result: &WorkflowV2Result) -> bool {
    result
        .data
        .get(HOST_FAULT_NO_VERDICT_MARKER)
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

/// How many calls in a row may fail without ever starting before the run
/// pauses (Issue 263; it used to stop the run).
///
/// A run-scoped counter, reset by any call that actually executes. The bound
/// exists because a never-ran failure costs no time: a generated loop handed
/// one will ask for the next attempt immediately, complete its whole bounded
/// budget with nothing executed, and move on to do the same to every task left.
/// It is a no-progress bound: consecutive dispatches that executed nothing.
/// Reaching it pauses the run with the evidence, so an operator can repair
/// the host's state and resume; it never fails the run.
#[derive(Debug, Clone)]
pub struct NeverStartedStreak {
    limit: usize,
    consecutive: usize,
}

impl Default for NeverStartedStreak {
    fn default() -> Self {
        Self::new(Self::DEFAULT_LIMIT)
    }
}

impl NeverStartedStreak {
    /// One never-ran failure is reported to the script as a failed stage; the
    /// next consecutive one pauses the run. Two rather than one because a
    /// single such failure is information the script should record against the
    /// stage it belongs to, and two rather than more because every extra one
    /// is another task's budget spent on nothing.
    pub const DEFAULT_LIMIT: usize = 2;

    pub fn new(limit: usize) -> Self {
        Self {
            limit: limit.max(1),
            consecutive: 0,
        }
    }

    /// A call executed. Whatever it decided, the host is working again.
    pub fn record_executed(&mut self) {
        self.consecutive = 0;
    }

    /// A call failed without ever starting.
    ///
    /// Returns true when the host must not hand the script another instant
    /// failure to charge against a task: it pauses the run instead.
    #[must_use]
    pub fn record_never_started(&mut self) -> bool {
        self.consecutive = self.consecutive.saturating_add(1);
        self.consecutive >= self.limit
    }

    pub fn consecutive(&self) -> usize {
        self.consecutive
    }
}

#[cfg(test)]
#[path = "host_fault_tests.rs"]
mod tests;
