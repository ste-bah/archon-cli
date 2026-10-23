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

/// Who an incomplete judge is owed to, set where it is produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum JudgeIncompleteKind {
    /// The judge's own replies gave no usable verdict (malformed, missing
    /// or empty decisions, a truncation it never completed): retried with
    /// the affected checks.
    AuthoredReply,
    /// The provider or the host failed (no reply, a stall, an unsupported
    /// or absent finish reason, a reply the host could not save): an
    /// operational outcome.
    Provider,
}

/// The judge could not complete its batch (Issue 260): an operational,
/// resumable outcome, never a verdict and never a candidate defect.
#[derive(Debug)]
pub(crate) struct JudgeIncomplete {
    pub(crate) kind: JudgeIncompleteKind,
    /// What happened, for display only: never classified.
    pub(crate) message: String,
    /// The entries the judge did decide in this call before it could not
    /// complete; every other entry it was asked about is still undecided.
    pub(crate) decided: Vec<archon_workflow::task_set_contract::AcceptanceCriterion>,
}

impl JudgeIncomplete {
    /// The provider or the host failed.
    pub(crate) fn provider(message: impl Into<String>) -> Self {
        Self::new(JudgeIncompleteKind::Provider, message)
    }

    pub(crate) fn new(kind: JudgeIncompleteKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            decided: Vec::new(),
        }
    }

    /// The incomplete judge `error` carries, if any.
    pub(crate) fn caused(error: &anyhow::Error) -> Option<&Self> {
        error.chain().find_map(|cause| cause.downcast_ref::<Self>())
    }

    /// A judge-authored reply can be retried with the affected checks. A
    /// stalled provider call or provider error remains a host outcome.
    pub(crate) fn is_authored_reply(&self) -> bool {
        self.kind == JudgeIncompleteKind::AuthoredReply
    }
}

impl std::fmt::Display for JudgeIncomplete {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "operational: acceptance judge incomplete, resumable: {}",
            self.message
        )
    }
}

impl std::error::Error for JudgeIncomplete {}

#[cfg(test)]
#[path = "workflow_task_set_judge_incomplete_tests.rs"]
pub(crate) mod tests;
