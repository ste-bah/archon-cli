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
use crate::v2::WorkflowV2Result;
use crate::v2::script::failed_v2_result;

/// The marker this crate stamps on a result for a call that never executed.
pub const HOST_FAULT_NO_VERDICT_MARKER: &str = "host_fault_no_verdict";

/// The marker the frozen script primitives already key their attempt refund
/// on. Their pool is documented as "an attempt burned by something that says
/// nothing about the work", which is exactly what a call that never ran is, so
/// a never-ran result joins that pool instead of inventing a second one the
/// primitives cannot see.
pub const NO_VERDICT_REFUND_MARKER: &str = "transport_failure_no_verdict";

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

/// How many calls in a row may fail without ever starting before the run stops.
///
/// A run-scoped counter, reset by any call that actually executes. The bound
/// exists because a never-ran failure costs no time: a generated loop handed
/// one will ask for the next attempt immediately, complete its whole bounded
/// budget with nothing executed, and move on to do the same to every task left.
/// Refusing to hand the script a second consecutive one turns that into a
/// single failed stage and a run that stops with the reason still legible.
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
    /// next consecutive one stops the run. Two rather than one because a
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
    /// Returns true when the host must refuse to continue — propagate the
    /// error and end the run — instead of handing the script another instant
    /// failure to charge against a task.
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
