pub(super) fn judge_error_class(error: &str) -> &'static str {
    if error.contains("malformed batched JSON") {
        "malformed_json"
    } else if error.contains("duplicated decision") {
        "duplicate_decision"
    } else if error.contains("decision ids do not match") {
        "decision_ids"
    } else if error.contains("left decisions missing") {
        "missing_decisions"
    } else if error.contains("left 'counterexample' empty") {
        "empty_counterexample"
    } else if error.contains("left 'reason' empty") {
        "empty_reason"
    } else if error.contains("unsupported stop reason") || error.contains("no finish reason") {
        "finish_reason"
    } else {
        "invalid_judgment"
    }
}

/// The judge could not complete its batch (Issue 260): an operational,
/// resumable outcome, never a verdict and never a candidate defect.
#[derive(Debug)]
pub(crate) struct JudgeIncomplete(pub(crate) String);

impl JudgeIncomplete {
    /// The incomplete judge `error` carries, if any.
    pub(crate) fn caused(error: &anyhow::Error) -> Option<&Self> {
        error.chain().find_map(|cause| cause.downcast_ref::<Self>())
    }
}

impl std::fmt::Display for JudgeIncomplete {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "operational: acceptance judge incomplete, resumable: {}",
            self.0
        )
    }
}

impl std::error::Error for JudgeIncomplete {}
