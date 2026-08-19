//! Which messages are actually filling the context window (#189 Phase 3).
//!
//! Compaction already knows *that* the window is 82% full — it reads the
//! provider's own `context_input_tokens`. What it has never known is *which*
//! messages account for that, so its only move is to summarise everything
//! rather than drop the three expensive things.
//!
//! Two problems stood between the estimate and being useful:
//!
//! 1. `estimate_message_tokens` is `len / 4`. Summed, it is close enough to
//!    trigger compaction; per message, an uncalibrated guess is a poor basis
//!    for deciding what to throw away.
//! 2. The one authoritative number, `last_known_context_tokens`, is reset to 0
//!    on compaction (`autocompact_agent.rs`, `compaction.rs`) — exactly when
//!    knowing the new size matters most.
//!
//! The fix for both is the same: reconcile the estimate against the provider's
//! number once, and keep the ratio. A tokenizer's bytes-per-token is a property
//! of the tokenizer and the text, not of the message set, so the factor
//! survives a compaction that clears the anchor.

/// One message's share of the context window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenSurfaceNode {
    /// Index into the conversation's message list.
    pub message_index: usize,
    /// Calibrated token estimate for that message.
    pub estimated_tokens: u64,
}

/// Per-message attribution for the live message set.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TokenSurface {
    nodes: Vec<TokenSurfaceNode>,
    calibration: Calibration,
}

/// The correction applied to raw `len / 4` estimates.
///
/// Starts as an identity factor and is refined the first time a real usage
/// number arrives. Held separately from the nodes so it outlives them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Calibration {
    factor: f64,
    /// Whether a provider number has ever been observed.
    calibrated: bool,
}

impl Default for Calibration {
    fn default() -> Self {
        Self {
            factor: 1.0,
            calibrated: false,
        }
    }
}

impl Calibration {
    /// Clamp bounds for the correction factor.
    ///
    /// A ratio outside this range is not a tokenizer difference; it means the
    /// usage number and the message set did not describe the same thing — a
    /// reset anchor, a mid-flight request, a provider counting the system
    /// prompt differently. Applying it would make every later estimate worse
    /// than the uncorrected guess.
    const MIN_FACTOR: f64 = 0.25;
    const MAX_FACTOR: f64 = 4.0;

    #[must_use]
    pub fn factor(self) -> f64 {
        self.factor
    }

    #[must_use]
    pub fn is_calibrated(self) -> bool {
        self.calibrated
    }

    /// Fold in a real provider count for a known raw estimate.
    ///
    /// Returns whether the observation was accepted.
    pub fn observe(&mut self, raw_estimate: u64, actual_tokens: u64) -> bool {
        if raw_estimate == 0 || actual_tokens == 0 {
            return false;
        }
        let ratio = actual_tokens as f64 / raw_estimate as f64;
        if !(Self::MIN_FACTOR..=Self::MAX_FACTOR).contains(&ratio) {
            return false;
        }
        self.factor = ratio;
        self.calibrated = true;
        true
    }

    /// Apply the correction to a raw estimate.
    #[must_use]
    pub fn apply(self, raw_estimate: u64) -> u64 {
        (raw_estimate as f64 * self.factor).round() as u64
    }
}

impl TokenSurface {
    /// Build attribution for a message set, carrying the calibration forward.
    #[must_use]
    pub fn build(messages: &[serde_json::Value], calibration: Calibration) -> Self {
        let nodes = messages
            .iter()
            .enumerate()
            .map(|(message_index, message)| TokenSurfaceNode {
                message_index,
                estimated_tokens: calibration
                    .apply(super::autocompact::estimate_message_tokens(message)),
            })
            .collect();
        Self { nodes, calibration }
    }

    #[must_use]
    pub fn nodes(&self) -> &[TokenSurfaceNode] {
        &self.nodes
    }

    #[must_use]
    pub fn calibration(&self) -> Calibration {
        self.calibration
    }

    /// Total attributed tokens.
    #[must_use]
    pub fn total(&self) -> u64 {
        self.nodes.iter().map(|node| node.estimated_tokens).sum()
    }

    /// The messages accounting for the largest share, biggest first.
    ///
    /// This is the question compaction could not previously ask: not "how full
    /// is the window" but "what is filling it".
    #[must_use]
    pub fn top_contributors(&self, limit: usize) -> Vec<TokenSurfaceNode> {
        let mut ranked = self.nodes.clone();
        // Ties broken by index so the order is stable between refreshes.
        ranked.sort_by(|a, b| {
            b.estimated_tokens
                .cmp(&a.estimated_tokens)
                .then(a.message_index.cmp(&b.message_index))
        });
        ranked.truncate(limit);
        ranked
    }

    /// The fewest messages that together account for `fraction` of the surface.
    ///
    /// Pruning wants the smallest set worth acting on, not a fixed top-N: a
    /// conversation with one enormous message and forty small ones should
    /// return one entry, not ten.
    #[must_use]
    pub fn nodes_covering(&self, fraction: f64) -> Vec<TokenSurfaceNode> {
        let total = self.total();
        if total == 0 || fraction <= 0.0 {
            return Vec::new();
        }
        let target = (total as f64 * fraction.min(1.0)).ceil() as u64;
        let mut accumulated = 0;
        let mut chosen = Vec::new();
        for node in self.top_contributors(self.nodes.len()) {
            chosen.push(node);
            accumulated += node.estimated_tokens;
            if accumulated >= target {
                break;
            }
        }
        chosen
    }

    /// Reconcile against a real provider count and rebuild.
    ///
    /// `messages` must be the set the count describes; a stale set produces a
    /// ratio that is rejected by the clamp rather than silently applied.
    pub fn reconcile(&mut self, messages: &[serde_json::Value], actual_tokens: u64) -> bool {
        let raw = super::autocompact::estimate_messages_tokens(messages);
        let accepted = self.calibration.observe(raw, actual_tokens);
        *self = Self::build(messages, self.calibration);
        accepted
    }
}

impl super::types::ConversationState {
    /// Per-message attribution for the current message set.
    ///
    /// Built on demand rather than cached: the message list changes several
    /// times a turn, and a stale surface is worse than none.
    #[must_use]
    pub fn token_surface(&self) -> TokenSurface {
        TokenSurface::build(&self.messages, self.token_calibration)
    }

    /// Fold a real provider count into the calibration.
    ///
    /// The count covers the whole request — system prompt, tool schemas and
    /// memory injections included — while the estimate covers messages only.
    /// The factor therefore absorbs that fixed overhead and spreads it across
    /// messages proportionally, which is what makes the surface sum to the
    /// reported context size. Relative ranking between messages is unaffected,
    /// since the factor is uniform.
    pub fn reconcile_token_surface(&mut self, context_input_tokens: u64) -> bool {
        let raw = super::autocompact::estimate_messages_tokens(&self.messages);
        self.token_calibration.observe(raw, context_input_tokens)
    }
}

#[cfg(test)]
#[path = "token_surface_tests.rs"]
mod tests;
