//! Proactive request-pressure helpers for the subagent run loop.
//!
//! Split from `request_round.rs` for the 500-line ceiling; the two are one
//! unit and the caller lives there.

use super::super::SubagentRunner;
use super::message_history::MessageHistory;

/// Re-arm margins for a proactive compaction that FAILED.
///
/// A failed attempt must not be retried against a history that has barely
/// changed: the summariser call costs a full-context inference pass and the
/// wall clock to match, so retrying every turn against the same source turns
/// one failure into a hang. It must also not disarm the guard for the rest of
/// the run, which is the bug this replaces.
///
/// Growth must clear BOTH margins. The absolute one stops a trickle of small
/// tool results from re-arming immediately; the relative one keeps the gap
/// proportional once a history is already large.
pub(super) const REARM_ABSOLUTE_TOKENS: u64 = 4_096;
pub(super) const REARM_RELATIVE_GROWTH: f64 = 1.05;

/// Whether proactive compaction may be attempted at this size.
///
/// `watermark` is `None` when nothing has failed yet, or after a compaction
/// succeeded — a success re-arms unconditionally, because the history it
/// produced is the new baseline and the pressure threshold alone decides when
/// to act on it again.
///
/// THE BUG THIS REPLACES: the field was `proactive_attempted: bool`, set once
/// and reset nowhere in the codebase, on a `PressureState` built OUTSIDE the
/// turn loop. So a subagent got exactly one proactive compaction per run, and
/// on this deployment that one attempt was made with the reasoning model and
/// returned an empty summary — after which nothing reclaimed context again.
/// A read-only discovery branch grew to 416,252 tokens against a 262,144
/// window; the server rejected it mid-stream and the branch died with
/// "stream ended before message_stop". A 1M window had hidden this for months.
pub(super) fn proactive_rearmed(watermark: Option<u64>, trigger_tokens: u64) -> bool {
    match watermark {
        None => true,
        Some(mark) => {
            trigger_tokens >= mark.saturating_add(REARM_ABSOLUTE_TOKENS)
                && (trigger_tokens as f64) >= (mark as f64) * REARM_RELATIVE_GROWTH
        }
    }
}

/// Reclaim context by arithmetic before spending a model call on it.
///
/// `[prune]` collapses repeated reads of a path nothing wrote to in between,
/// replaces failed tool results that later succeeded, and points spilled
/// results at their file. No message or block is ever removed, so this is
/// lossless in a way summarisation is not.
///
/// Running it here mirrors what the main agent already does in
/// `prune_agent::prune_context_mechanically`; the subagent path had no route
/// to it at all — `crates/archon-core/src/subagent` did not mention prune.
/// That matters most exactly when summarisation cannot help: once a history
/// exceeds the window, the summariser call carries that same oversized history
/// and fails for the same reason, while an arithmetic pass has no window.
///
/// Read-only audit branches are the ideal case — they re-read the same paths
/// and never write, so the intervening-write check that makes `repeated_reads`
/// sound can never veto it.
///
/// Returns the tokens reclaimed, or 0 when the pass is disabled or found
/// nothing.
pub(super) fn prune_history_mechanically(messages: &mut MessageHistory) -> u64 {
    let config = match crate::config::load_config() {
        Ok(loaded) => loaded.prune,
        Err(_) => return 0,
    };
    if !config.enabled || !config.any_rule_enabled() {
        return 0;
    }
    let before = messages.estimated_tokens();
    let outcome = crate::agent::prune::prune_mechanical(messages.as_slice(), config);
    if !outcome.reclaimed_anything() {
        return 0;
    }
    let rules = outcome.rules_fired.join(",");
    messages.replace(outcome.messages);
    let after = messages.estimated_tokens();
    let reclaimed = before.saturating_sub(after);
    tracing::info!(
        prune.rules = %rules,
        prune.bytes_reclaimed = outcome.bytes_reclaimed,
        tokens_before = before,
        tokens_after = after,
        scope = "subagent",
        "subagent context pruned mechanically"
    );
    reclaimed
}

pub(super) async fn compact_proactively(
    runner: &SubagentRunner,
    messages: &mut MessageHistory,
    auto_compact: &mut crate::agent::AutoCompactState,
    last_known_context_tokens: &mut u64,
    telemetry: &crate::agent::autocompact::CompactionTelemetry,
    action: crate::agent::CompactAction,
    failure_message: &str,
) -> bool {
    auto_compact.compact_in_flight = true;
    // A reasoning model answers a summarisation prompt with `reasoning_content`
    // and no text, so the summary comes back empty and the attempt fails. The
    // reactive path resolves `[context] compaction_model` for exactly this
    // (stream_round_recovery.rs); this path was left on `runner.model`, so the
    // one attempt each subagent got was made with the model least able to
    // produce a summary. Falls back to the active model when none is
    // configured, preserving prior behaviour.
    let available_models = runner.provider.models();
    let available: Vec<&str> = available_models
        .iter()
        .map(|model| model.id.as_str())
        .collect();
    let summary_model = crate::agent::autocompact::resolve_compaction_model(
        runner.agent_config.context.compaction_model.as_deref(),
        None,
        &runner.model,
        &available,
    )
    .model;
    match crate::agent::autocompact::compact_json_messages_with_provider(
        runner.provider.as_ref(),
        &summary_model,
        messages.as_slice(),
        action,
        false,
        runner.agent_config.runtime_attribution_extra(
            "compaction",
            "subagent_auto_compaction",
            None,
            None,
            None,
        ),
    )
    .await
    {
        Ok((
            crate::agent::autocompact::CompactionOutcome::Compacted {
                after_estimated_tokens,
                ..
            },
            compacted,
        )) => {
            messages.replace(compacted);
            *last_known_context_tokens = 0;
            auto_compact.on_success(after_estimated_tokens);
            true
        }
        Ok((crate::agent::autocompact::CompactionOutcome::Skipped { .. }, _)) => {
            auto_compact.on_cancel();
            false
        }
        Err(crate::agent::autocompact::CompactionError::Cancelled) => {
            auto_compact.on_cancel();
            tracing::debug!(
                compaction.outcome = "cancelled",
                provider_family = telemetry.provider_family,
                wire_shape = telemetry.wire_shape,
                native_context_window = telemetry.native_context_window,
                runtime_context_budget = telemetry.runtime_context_budget,
                context_source = telemetry.context_source,
                compaction_backend = telemetry.compaction_backend,
                actor = %runner.activity_actor_id.as_deref().unwrap_or("subagent"),
                "proactive subagent compaction cancelled"
            );
            false
        }
        Err(error) => {
            auto_compact.on_failure(&error);
            tracing::warn!(
                compaction.outcome = "auto_failed",
                provider_family = telemetry.provider_family,
                wire_shape = telemetry.wire_shape,
                native_context_window = telemetry.native_context_window,
                runtime_context_budget = telemetry.runtime_context_budget,
                context_source = telemetry.context_source,
                compaction_backend = telemetry.compaction_backend,
                actor = %runner.activity_actor_id.as_deref().unwrap_or("subagent"),
                consecutive_failures = auto_compact.consecutive_failures,
                breaker_tripped = auto_compact.disabled,
                error = %error,
                "{failure_message}",
            );
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_state_is_armed() {
        assert!(proactive_rearmed(None, 0));
        assert!(proactive_rearmed(None, 500_000));
    }

    /// The bug: one failure must not disarm the guard for the rest of the run.
    #[test]
    fn a_failed_attempt_rearms_once_the_history_grows_past_both_margins() {
        assert!(proactive_rearmed(Some(100_000), 105_000));
    }

    #[test]
    fn a_failed_attempt_does_not_retry_against_an_unchanged_history() {
        assert!(!proactive_rearmed(Some(100_000), 100_000));
        assert!(!proactive_rearmed(Some(100_000), 100_100));
    }

    /// Both margins must clear, not either. At 400k the absolute margin alone
    /// would re-arm on a rounding error.
    #[test]
    fn the_relative_margin_still_binds_on_a_large_history() {
        assert!(!proactive_rearmed(Some(400_000), 410_000));
        assert!(proactive_rearmed(Some(400_000), 420_000));
    }

    /// And the absolute margin still binds on a small one, where 5% is noise.
    #[test]
    fn the_absolute_margin_still_binds_on_a_small_history() {
        assert!(!proactive_rearmed(Some(10_000), 10_600));
        assert!(proactive_rearmed(Some(10_000), 14_096));
    }
}
