use super::*;

/// Adds what one stream event grew the round's usage by (`before` to
/// `after`, both from its [`archon_llm::usage::UsageAccumulator`]) to the
/// subagent's progress tracker. The accumulator owns the stream usage rule:
/// `message_delta` counts are cumulative for their message (Issue 424), and a
/// count only a delta reports, or reports higher than `message_start` did, is
/// counted (Issue 369). A resent round starts a fresh accumulator; the
/// tracker keeps what the discarded attempt was billed.
pub(super) fn record_usage_growth(
    runner: &SubagentRunner,
    before: &archon_llm::types::Usage,
    after: &archon_llm::types::Usage,
) {
    let grew = |after: u64, before: u64| after.saturating_sub(before);
    let input = grew(after.input_tokens, before.input_tokens);
    let output = grew(after.output_tokens, before.output_tokens);
    let cache_creation = grew(
        after.cache_creation_input_tokens,
        before.cache_creation_input_tokens,
    );
    let cache_read = grew(
        after.cache_read_input_tokens,
        before.cache_read_input_tokens,
    );
    let newly_available = (after.input_tokens_available && !before.input_tokens_available)
        || (after.output_tokens_available && !before.output_tokens_available);
    if input == 0 && output == 0 && cache_creation == 0 && cache_read == 0 && !newly_available {
        return;
    }
    if let Some(ref tracker) = runner.progress
        && let Ok(mut guard) = tracker.lock()
    {
        guard.cumulative_input_tokens += input;
        guard.cumulative_output_tokens += output;
        guard.input_tokens_available |= after.input_tokens_available;
        guard.output_tokens_available |= after.output_tokens_available;
        guard.cumulative_cache_creation_tokens += cache_creation;
        guard.cumulative_cache_read_tokens += cache_read;
        guard.last_update = chrono::Utc::now();
    }
}
