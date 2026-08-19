//! Trying arithmetic before asking a model (#189 Phase 8).
//!
//! Split from `autocompact_agent.rs` so that file stays under the 500-line
//! ceiling.

use super::*;
use crate::agent::autocompact::CompactionTelemetry;

impl Agent {
    /// Reclaim what can be reclaimed mechanically, and report whether that was
    /// enough to leave the compaction threshold behind.
    ///
    /// Returns `true` only when the model call can be skipped outright. A
    /// partial saving still rewrites the history — those bytes are gone either
    /// way — but the summary still runs, because stopping half-way would leave
    /// the window over threshold and the next turn would trigger again.
    ///
    /// `force` bypasses the skip: a forced compaction was asked for, and
    /// answering it with "pruning was sufficient" would ignore the request.
    pub(in crate::agent) fn prune_context_mechanically(
        &mut self,
        telemetry: &CompactionTelemetry,
        force: bool,
    ) -> bool {
        // Read at compaction time rather than held on `AgentConfig`, which is
        // constructed in too many places to thread a new field through for a
        // decision taken once every few dozen turns. Same approach as spill.
        let config = crate::config::load_config()
            .map(|loaded| loaded.prune)
            .unwrap_or_default();
        if !config.enabled || !config.any_rule_enabled() {
            return false;
        }
        let outcome = prune::prune_mechanical(&self.state.messages, config);
        if !outcome.reclaimed_anything() {
            return false;
        }

        self.state.messages = outcome.messages;
        self.invalidate_memory_injector_cache();

        let window = telemetry
            .runtime_context_budget
            .saturating_sub(self.config.context.output_reserve_tokens);
        let threshold = self.config.context.compact_threshold;
        // Re-estimate from the rewritten history. The provider's number
        // described the messages as they were a moment ago and is now stale in
        // exactly the direction that matters.
        let after = autocompact::trigger_tokens(&self.state.messages);
        let cleared = !force && window > 0 && (after as f32 / window as f32) < threshold;

        tracing::info!(
            compaction.reason = "mechanical_prune",
            compaction.bytes_reclaimed = outcome.bytes_reclaimed,
            compaction.rules_fired = outcome.rules_fired.join(","),
            compaction.after_estimated_tokens = after,
            compaction.window = window,
            compaction.model_call_skipped = cleared,
            provider_family = telemetry.provider_family,
            "mechanical context prune"
        );

        if cleared {
            // The anchor described the pre-prune history; leaving it in place
            // would keep reporting a window that is no longer that full.
            self.state.last_known_context_tokens = 0;
            self.state.auto_compact.on_success(after);
        }
        cleared
    }
}

#[cfg(test)]
#[path = "prune_agent_tests.rs"]
mod tests;
