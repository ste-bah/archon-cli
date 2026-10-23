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

    /// A judge-authored reply can be retried with the affected checks. A
    /// stalled provider call or provider error remains a host outcome.
    pub(crate) fn is_authored_reply(&self) -> bool {
        [
            "malformed batched JSON",
            "duplicated decision",
            "decision ids do not match",
            "left decisions missing",
            "extra decision",
            "unknown verdict",
            "invalid verdict",
            "left 'counterexample' empty",
            "left 'reason' empty",
            "continuation ended normally with the document",
            "reply was truncated",
            "reply grew past",
            "continuation added no token",
            "continuation contradicts",
            "replies; the last",
        ]
        .iter()
        .any(|defect| self.0.contains(defect))
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

#[cfg(test)]
mod tests {
    use super::JudgeIncomplete;

    #[test]
    fn only_authored_reply_exhaustion_is_subject_retryable() {
        assert!(JudgeIncomplete("3 replies; malformed batched JSON".into()).is_authored_reply());
        assert!(
            JudgeIncomplete("the continuation ended normally with the document still open".into())
                .is_authored_reply()
        );
        assert!(
            !JudgeIncomplete("the provider gave no reply: disconnected".into()).is_authored_reply()
        );
        assert!(
            !JudgeIncomplete("a judge call stalled after 30s with no provider progress".into())
                .is_authored_reply()
        );
        assert!(
            !JudgeIncomplete("usable redacted continuation could not be persisted".into())
                .is_authored_reply()
        );
        for provider_fault in [
            "unsupported stop reason 'lengthy'",
            "returned no finish reason",
            "the provider gave no reply: disconnected",
            "a judge call stalled with no provider progress",
            "the usable redacted continuation could not be persisted",
        ] {
            assert!(
                !JudgeIncomplete(provider_fault.into()).is_authored_reply(),
                "provider or adapter fault was classified as authored: {provider_fault}"
            );
        }
        for authored_reply in [
            "malformed batched JSON",
            "duplicated decision id",
            "decision ids do not match",
            "left decisions missing",
            "left 'counterexample' empty",
            "left 'reason' empty",
            "unknown verdict value",
        ] {
            assert!(
                JudgeIncomplete(authored_reply.into()).is_authored_reply(),
                "judge-authored defect was not retryable: {authored_reply}"
            );
        }
    }
}
