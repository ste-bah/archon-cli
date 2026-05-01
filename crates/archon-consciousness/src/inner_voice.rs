//! Inner Voice / Consciousness State Tracking.
//!
//! Tracks confidence, energy, focus, struggles, and successes across
//! the lifetime of a session. Produces a prompt block for injection
//! into the system prompt and supports snapshot/restore for compaction.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Snapshot (serializable)
// ---------------------------------------------------------------------------

/// Serializable snapshot of the inner voice state for compaction persistence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InnerVoiceSnapshot {
    pub confidence: f32,
    pub energy: f32,
    pub focus: String,
    pub struggles: Vec<String>,
    pub successes: Vec<String>,
    pub turn_count: u32,
    pub corrections_received: u32,
}

// ---------------------------------------------------------------------------
// InnerVoice
// ---------------------------------------------------------------------------

/// Mutable consciousness state that evolves over a session.
#[derive(Debug, Clone)]
pub struct InnerVoice {
    /// Current confidence level (0.0–1.0, default 0.7).
    pub confidence: f32,
    /// Current energy level (0.0–1.0, default 1.0). Decays each turn.
    pub energy: f32,
    /// Current area of focus.
    pub focus: String,
    /// Areas with repeated failures (>= 3 consecutive failures).
    pub struggles: Vec<String>,
    /// Areas with consistent success.
    pub successes: Vec<String>,
    /// Number of completed turns.
    pub turn_count: u32,
    /// Number of user corrections received.
    pub corrections_received: u32,
    /// Per-tool failure counts (private; not serialized to prompt).
    tool_failure_counts: HashMap<String, u32>,
    /// Energy decay multiplier applied each turn (default 0.98).
    energy_decay_rate: f32,
}

impl InnerVoice {
    /// Create a new `InnerVoice` with default state.
    pub fn new() -> Self {
        Self {
            confidence: 0.7,
            energy: 1.0,
            focus: String::new(),
            struggles: Vec::new(),
            successes: Vec::new(),
            turn_count: 0,
            corrections_received: 0,
            tool_failure_counts: HashMap::new(),
            energy_decay_rate: 0.98,
        }
    }

    /// Create a new `InnerVoice` with a custom energy decay rate.
    pub fn with_decay_rate(decay_rate: f32) -> Self {
        let mut iv = Self::new();
        iv.energy_decay_rate = decay_rate;
        iv
    }

    /// Record a successful tool invocation.
    ///
    /// * Increases confidence by 0.02 (capped at 1.0).
    /// * Adds the tool to `successes` if not already present.
    /// * Updates focus to the tool name.
    /// * Resets the failure counter for this tool.
    pub fn on_tool_success(&mut self, tool_name: &str) {
        self.confidence = (self.confidence + 0.02).clamp(0.0, 1.0);

        if !self.successes.contains(&tool_name.to_string()) {
            self.successes.push(tool_name.to_string());
        }

        self.focus = tool_name.to_string();
        self.tool_failure_counts.insert(tool_name.to_string(), 0);
    }

    /// Record a failed tool invocation.
    ///
    /// * Decreases confidence by 0.05 (floored at 0.0).
    /// * Increments the per-tool failure counter; if it reaches 3, adds
    ///   the tool to `struggles`.
    /// * Updates focus to the tool name.
    pub fn on_tool_failure(&mut self, tool_name: &str) {
        self.confidence = (self.confidence - 0.05).clamp(0.0, 1.0);

        let count = self
            .tool_failure_counts
            .entry(tool_name.to_string())
            .or_insert(0);
        *count += 1;

        if *count >= 3 && !self.struggles.contains(&tool_name.to_string()) {
            self.struggles.push(tool_name.to_string());
        }

        self.focus = tool_name.to_string();
    }

    /// Record a user correction.
    ///
    /// Decreases confidence by 0.1 (floored at 0.0) and increments the
    /// correction counter.
    pub fn on_user_correction(&mut self) {
        self.confidence = (self.confidence - 0.1).clamp(0.0, 1.0);
        self.corrections_received += 1;
    }

    /// Record the completion of a conversational turn.
    ///
    /// Increments the turn counter and applies energy decay (energy *= 0.98,
    /// clamped to 0.0–1.0).
    pub fn on_turn_complete(&mut self) {
        self.turn_count += 1;
        self.energy = (self.energy * self.energy_decay_rate).clamp(0.0, 1.0);
    }

    /// Produce a serializable snapshot for compaction persistence.
    pub fn on_compaction(&self) -> InnerVoiceSnapshot {
        InnerVoiceSnapshot {
            confidence: self.confidence,
            energy: self.energy,
            focus: self.focus.clone(),
            struggles: self.struggles.clone(),
            successes: self.successes.clone(),
            turn_count: self.turn_count,
            corrections_received: self.corrections_received,
        }
    }

    /// Restore state from a previously persisted snapshot.
    pub fn from_snapshot(snapshot: InnerVoiceSnapshot) -> Self {
        Self {
            confidence: snapshot.confidence,
            energy: snapshot.energy,
            focus: snapshot.focus,
            struggles: snapshot.struggles,
            successes: snapshot.successes,
            turn_count: snapshot.turn_count,
            corrections_received: snapshot.corrections_received,
            tool_failure_counts: HashMap::new(),
            energy_decay_rate: 0.98,
        }
    }

    /// Format the current state as an `<inner_voice>` XML block suitable
    /// for injection into the system prompt.
    pub fn to_prompt_block(&self) -> String {
        let struggles_str = if self.struggles.is_empty() {
            "none".to_string()
        } else {
            self.struggles.join(", ")
        };

        let successes_str = if self.successes.is_empty() {
            "none".to_string()
        } else {
            self.successes.join(", ")
        };

        let focus_str = if self.focus.is_empty() {
            "none"
        } else {
            &self.focus
        };

        format!(
            "<inner_voice>\n\
             Confidence: {:.2}\n\
             Energy: {:.2}\n\
             Focus: {}\n\
             Struggles: {}\n\
             Successes: {}\n\
             Turns: {}\n\
             Corrections: {}\n\
             </inner_voice>",
            self.confidence,
            self.energy,
            focus_str,
            struggles_str,
            successes_str,
            self.turn_count,
            self.corrections_received,
        )
    }

    /// Produce a [`SessionStats`](crate::persistence::SessionStats) summary
    /// of the current session state.
    ///
    /// `start_confidence` is the confidence level at session start (before any
    /// turns). `duration_secs` is the wall-clock session duration.
    pub fn to_session_stats(
        &self,
        start_confidence: f32,
        duration_secs: u64,
    ) -> crate::persistence::SessionStats {
        let total_tool_calls: u32 =
            self.tool_failure_counts.values().copied().sum::<u32>() + self.successes.len() as u32;
        let total_tool_failures: u32 = self.tool_failure_counts.values().sum();

        crate::persistence::SessionStats {
            total_turns: self.turn_count,
            total_corrections: self.corrections_received,
            total_tool_calls,
            total_tool_failures,
            confidence_start: start_confidence,
            confidence_end: self.confidence,
            energy_end: self.energy,
            top_struggles: self.struggles.clone(),
            top_successes: self.successes.clone(),
            duration_secs,
        }
    }

    /// Check whether the inner voice feature is enabled via config.
    pub fn is_enabled(config_enabled: bool) -> bool {
        config_enabled
    }
}

impl Default for InnerVoice {
    fn default() -> Self {
        Self::new()
    }
}
