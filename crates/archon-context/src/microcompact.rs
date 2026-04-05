use crate::boundary::{CompactBoundary, CompactionStrategy};
use crate::messages::{ContextMessage, total_estimated_tokens};

/// Microcompact: summarize the oldest 30 % of messages, keep recent verbatim.
///
/// Triggered at ~60 % context usage.  The caller is responsible for generating
/// the `summary_text` via an LLM call; this function only manipulates the
/// message list.
///
/// `preserve_recent` is the number of recent *turns* (user+assistant pairs) to
/// keep verbatim.  If there are not enough messages to compact (i.e. the total
/// message count is at most `preserve_recent * 2`), the original list is
/// returned unchanged and the boundary reports zero tokens removed.
pub fn microcompact_messages(
    messages: &[ContextMessage],
    summary_text: &str,
    preserve_recent: usize,
) -> (Vec<ContextMessage>, CompactBoundary) {
    let min_required = preserve_recent * 2;
    if messages.len() <= min_required {
        let boundary = CompactBoundary {
            summary: String::new(),
            tokens_removed: 0,
            tokens_remaining: total_estimated_tokens(messages),
            strategy: CompactionStrategy::Micro,
            timestamp: chrono::Utc::now(),
        };
        return (messages.to_vec(), boundary);
    }

    // Calculate 30 % of messages from the start to summarize.
    let summarize_count = (messages.len() as f64 * 0.30).ceil() as usize;
    // Ensure we keep at least `preserve_recent * 2` messages verbatim.
    let split = summarize_count.min(messages.len().saturating_sub(min_required));
    if split == 0 {
        let boundary = CompactBoundary {
            summary: String::new(),
            tokens_removed: 0,
            tokens_remaining: total_estimated_tokens(messages),
            strategy: CompactionStrategy::Micro,
            timestamp: chrono::Utc::now(),
        };
        return (messages.to_vec(), boundary);
    }

    let removed_tokens = total_estimated_tokens(&messages[..split]);
    let remaining_tokens = total_estimated_tokens(&messages[split..]);

    let boundary = CompactBoundary {
        summary: summary_text.to_string(),
        tokens_removed: removed_tokens,
        tokens_remaining: remaining_tokens,
        strategy: CompactionStrategy::Micro,
        timestamp: chrono::Utc::now(),
    };

    let mut result = Vec::with_capacity(2 + messages.len() - split);
    // Summary replaces the removed messages.
    result.push(ContextMessage::user(summary_text));
    // Boundary marker.
    result.push(boundary.to_message());
    // Remaining messages kept verbatim.
    result.extend_from_slice(&messages[split..]);

    (result, boundary)
}
