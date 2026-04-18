//! Session statistics tracking for the Archon TUI.
//!
//! Tracks message counts, agent interactions, token usage, and command history
//! for the current session.

use archon_llm::TokenUsage;
use std::time::Duration;

/// Statistics for the current session.
///
/// Collects metrics about messages, agents, tokens, and commands
/// to display in the TUI status bar.
#[derive(Debug, Clone, Default)]
pub struct SessionStats {
    /// Number of messages exchanged in this session.
    pub message_count: u64,
    /// Number of agents active in this session.
    pub agent_count: u64,
    /// Token usage statistics for the session.
    pub token_usage: TokenUsage,
    /// Time elapsed since session start.
    pub elapsed: Duration,
    /// Estimated cost in USD, if calculable.
    pub estimated_cost_usd: Option<f64>,
    /// Recent commands executed in this session.
    pub recent_commands: Vec<String>,
    /// Recent agents active in this session.
    pub recent_agents: Vec<String>,
    /// Recent tools invoked in this session.
    pub recent_tools: Vec<String>,
}

impl SessionStats {
    /// Creates an empty SessionStats with all fields set to zero/empty values.
    pub fn empty() -> Self {
        Self {
            message_count: 0,
            agent_count: 0,
            token_usage: TokenUsage::default(),
            elapsed: Duration::ZERO,
            estimated_cost_usd: None,
            recent_commands: Vec::new(),
            recent_agents: Vec::new(),
            recent_tools: Vec::new(),
        }
    }

    /// Returns true if this SessionStats represents an empty/idle session.
    ///
    /// A session is considered empty when it has no messages, no agents,
    /// and no recorded tools.
    pub fn is_empty(&self) -> bool {
        self.message_count == 0
            && self.agent_count == 0
            && self.recent_commands.is_empty()
            && self.recent_agents.is_empty()
            && self.recent_tools.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_creates_zero_values() {
        let stats = SessionStats::empty();
        assert_eq!(stats.message_count, 0);
        assert_eq!(stats.agent_count, 0);
        assert_eq!(stats.elapsed, Duration::ZERO);
        assert!(stats.estimated_cost_usd.is_none());
        assert!(stats.recent_commands.is_empty());
        assert!(stats.recent_agents.is_empty());
        assert!(stats.recent_tools.is_empty());
    }

    #[test]
    fn test_is_empty_returns_true_for_empty_stats() {
        let stats = SessionStats::empty();
        assert!(stats.is_empty());
    }

    #[test]
    fn test_is_empty_returns_false_when_counts_nonzero() {
        let stats = SessionStats {
            message_count: 5,
            ..SessionStats::empty()
        };
        assert!(!stats.is_empty());
    }

    #[test]
    fn test_is_empty_returns_false_when_agents_present() {
        let stats = SessionStats {
            recent_agents: vec!["coder".to_string()],
            ..SessionStats::empty()
        };
        assert!(!stats.is_empty());
    }
}
