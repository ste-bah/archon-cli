use crate::boundary::{CompactBoundary, CompactionStrategy};
use crate::messages::{ContextMessage, total_estimated_tokens};

/// Count total turns in the message list.
///
/// A turn starts at each message with `role == "user"`.  Turn indices used
/// elsewhere in this module are **1-based**.
pub fn count_turns(messages: &[ContextMessage]) -> usize {
    messages.iter().filter(|m| m.role == "user").count()
}

/// Return the message-index ranges for each turn.
///
/// A turn begins at a user message and extends until (but not including) the
/// next user message or the end of the list.  The returned vec is 0-indexed
/// (vec index 0 = turn 1).
fn turn_ranges(messages: &[ContextMessage]) -> Vec<std::ops::Range<usize>> {
    let mut ranges = Vec::new();
    let mut start: Option<usize> = None;

    for (i, msg) in messages.iter().enumerate() {
        if msg.role == "user" {
            if let Some(s) = start {
                ranges.push(s..i);
            }
            start = Some(i);
        }
    }
    if let Some(s) = start {
        ranges.push(s..messages.len());
    }

    ranges
}

/// Snip: remove a specific range of turns without summarization.
///
/// `start_turn` and `end_turn` are **1-based, inclusive**.  A turn is defined
/// as a user message plus all following messages until the next user message.
///
/// Returns the resulting message list and a [`CompactBoundary`] placed at the
/// removal point.
pub fn snip_messages(
    messages: &[ContextMessage],
    start_turn: usize,
    end_turn: usize,
) -> Result<(Vec<ContextMessage>, CompactBoundary), String> {
    if start_turn == 0 {
        return Err("turn indices are 1-based; start_turn must be >= 1".into());
    }
    if start_turn > end_turn {
        return Err(format!(
            "invalid range: start_turn ({start_turn}) > end_turn ({end_turn})"
        ));
    }

    let ranges = turn_ranges(messages);
    let total_turns = ranges.len();

    if end_turn > total_turns {
        return Err(format!(
            "end_turn ({end_turn}) exceeds total turns ({total_turns})"
        ));
    }

    // Convert 1-based turn indices to 0-based range indices.
    let first_range = &ranges[start_turn - 1];
    let last_range = &ranges[end_turn - 1];
    let remove_start = first_range.start;
    let remove_end = last_range.end;

    let removed = &messages[remove_start..remove_end];
    let removed_tokens = total_estimated_tokens(removed);
    let kept_before = &messages[..remove_start];
    let kept_after = &messages[remove_end..];
    let remaining_tokens = total_estimated_tokens(kept_before) + total_estimated_tokens(kept_after);

    let boundary = CompactBoundary {
        summary: format!(
            "Snipped turns {start_turn}–{end_turn} ({} messages)",
            removed.len()
        ),
        tokens_removed: removed_tokens,
        tokens_remaining: remaining_tokens,
        strategy: CompactionStrategy::Snip,
        timestamp: chrono::Utc::now(),
    };

    let mut result = Vec::with_capacity(kept_before.len() + 1 + kept_after.len());
    result.extend_from_slice(kept_before);
    result.push(boundary.to_message());
    result.extend_from_slice(kept_after);

    Ok((result, boundary))
}
