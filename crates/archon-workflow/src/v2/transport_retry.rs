//! Re-ask a branch whose agent call died in transport.
//!
//! A dropped provider connection says nothing about the work. The branch had
//! no chance to produce a verdict, nothing landed, and the wave then reports no
//! completion and the run blocks.
//!
//! Shared by write branches and read-only fanout branches alike: it lived
//! inside `write/` first, which left verification branches with NO transport
//! retry at all — a provider outage permanently failed every verification
//! branch it touched while the sibling write branches would have been re-asked.
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
pub const MAX_TRANSPORT_RETRIES: usize = 3;

/// Did this branch die because the provider call failed, rather than because
/// the work was wrong?
pub fn is_transport_failure(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("agent transport failed")
        || lower.contains("response_failed")
        || lower.contains("connection closed")
        || lower.contains("connection reset")
        || lower.contains("stream ended unexpectedly")
}

/// A rejection that names the work is NOT transport, even when it travels
/// inside a stage-failure wrapper.
///
/// The second group is deterministic request-shape rejections: a prompt the
/// provider refuses for its SIZE will be refused identically on every re-ask,
/// so retrying it burns attempts and multiplies the compaction path's own
/// recovery requests (observed as a 2-request overflow fixture making 8).
/// The markers mirror `archon-llm`'s context-window classifier and its
/// `ContextWindowExceeded` display text.
pub fn is_content_rejection(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("entire patch is rejected")
        || lower.contains("outside declared")
        || lower.contains("changed files outside")
        || lower.contains("agent result failed validation")
        || lower.contains("context window")
        || lower.contains("context length")
        || lower.contains("maximum context")
        || lower.contains("prompt too long")
        || lower.contains("prompt is too long")
        || lower.contains("too many tokens")
        || lower.contains("request too large")
        || lower.contains("gate_rejected")
        // Compaction/pressure recovery giving up is deterministic for the
        // same request: the prompt that had no safe boundary or exhausted its
        // bounded recovery will do so identically on the re-ask.
        || lower.contains("no safe compaction boundary")
        || lower.contains("reactive subagent compaction failed")
        || lower.contains("request pressure recovery exhausted")
}

#[cfg(test)]
#[path = "transport_retry_tests.rs"]
mod tests;
