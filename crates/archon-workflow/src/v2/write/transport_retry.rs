//! Re-ask a write branch whose agent call died in transport.
//!
//! A dropped provider connection says nothing about the work. The branch had
//! no chance to produce a verdict, nothing landed, and the wave then reports no
//! completion and the run blocks.
//!
//! Observed twice in one morning, both ending the run:
//!
//! ```text
//! agent transport failed: subagent failed: HTTP error:
//!   response_failed: Codex response failed
//! ```
//!
//! `write_branch_error_kind` already calls this `Execution` rather than a
//! contract failure, and `is_recoverable_write_branch_timeout` already re-asks
//! the sibling case where the agent runs out of time. A connection that drops
//! is the same class — the provider failed, not the branch — and is retried on
//! the same terms.
//!
//! Matched on the error text alone, so it carries no task, provider or PRD
//! knowledge.

/// Transport is retried more freely than a content rejection: each attempt is
/// answering the same question of a provider that simply did not respond.
pub(super) const MAX_TRANSPORT_RETRIES: usize = 3;

/// Did this branch die because the provider call failed, rather than because
/// the work was wrong?
pub(super) fn is_transport_failure(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("agent transport failed")
        || lower.contains("response_failed")
        || lower.contains("connection closed")
        || lower.contains("connection reset")
        || lower.contains("stream ended unexpectedly")
}

/// A rejection that names the work is NOT transport, even when it travels
/// inside a stage-failure wrapper.
pub(super) fn is_content_rejection(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("entire patch is rejected")
        || lower.contains("outside declared")
        || lower.contains("changed files outside")
        || lower.contains("agent result failed validation")
}

#[cfg(test)]
#[path = "transport_retry_tests.rs"]
mod tests;
