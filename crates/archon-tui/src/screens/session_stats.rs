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

    // -------------------------------------------------------------------------
    // Tests for compute_stats (TDD - implementation pending)
    // -------------------------------------------------------------------------

    /// Temporary shim until TECH-TUI-OBSERVABILITY `ChannelMetrics` exists;
    /// then `ChannelMetrics` implements `StatsSource`.
    pub trait StatsSource {
        fn total_sent(&self) -> u64;
        fn total_drained(&self) -> u64;
        fn backlog_depth(&self) -> u64;
    }

    pub struct NullStats;

    impl StatsSource for NullStats {
        fn total_sent(&self) -> u64 {
            0
        }
        fn total_drained(&self) -> u64 {
            0
        }
        fn backlog_depth(&self) -> u64 {
            0
        }
    }

    /// Computes session statistics by aggregating data from the session store
    /// and runtime metrics source.
    ///
    /// # Arguments
    /// * `store` - Session store to fetch session data from
    /// * `session_id` - ID of the session to compute stats for
    /// * `metrics` - Runtime metrics source (send/drain counts, backlog depth)
    ///
    /// # Returns
    /// `SessionStats` with aggregated counts, token usage, elapsed time, and
    /// recent activity lists (capped at 10 entries each).
    ///
    /// Pricing table resolution is a follow-up: `estimated_cost_usd` is always `None`.
    pub fn compute_stats(
        _store: &archon_session::storage::SessionStore,
        _session_id: &str,
        _metrics: &dyn StatsSource,
    ) -> SessionStats {
        unimplemented!("compute_stats implementation pending")
    }

    #[test]
    fn test_compute_stats_empty_session_returns_zero_counts() {
        use archon_session::storage::SessionStore;

        // Create a temporary in-memory database
        let temp_dir = std::env::temp_dir();
        let db_path = temp_dir.join("test_compute_stats_empty.db");
        let store = SessionStore::open(&db_path).unwrap();

        // Create a session with no messages
        let session = store.create_session("/tmp", None, "claude-3-5-sonnet-4b").unwrap();
        let metrics = NullStats;

        let stats = compute_stats(&store, &session.id, &metrics);

        assert_eq!(stats.message_count, 0);
        assert_eq!(stats.agent_count, 0);
        // Cannot use assert_eq! on TokenUsage (no PartialEq) - check fields instead
        assert_eq!(stats.token_usage.input_tokens, 0);
        assert_eq!(stats.token_usage.output_tokens, 0);
        assert_eq!(stats.token_usage.cache_creation_input_tokens, 0);
        assert_eq!(stats.token_usage.cache_read_input_tokens, 0);
        assert!(stats.estimated_cost_usd.is_none());
        assert!(stats.recent_commands.is_empty());
        assert!(stats.recent_agents.is_empty());
        assert!(stats.recent_tools.is_empty());

        // Cleanup
        std::fs::remove_file(&db_path).ok();
    }

    #[test]
    fn test_compute_stats_counts_messages_and_agents() {
        use archon_session::storage::SessionStore;

        let temp_dir = std::env::temp_dir();
        let db_path = temp_dir.join("test_compute_stats_populated.db");
        let store = SessionStore::open(&db_path).unwrap();

        // Create a session
        let session = store.create_session("/tmp", None, "claude-3-5-sonnet-4b").unwrap();

        // Add messages with agent metadata (stored as JSON content)
        store.save_message(&session.id, 0, r#"{"role":"user","agent":"user"}"#).unwrap();
        store.save_message(&session.id, 1, r#"{"role":"assistant","agent":"coder"}"#).unwrap();
        store.save_message(&session.id, 2, r#"{"role":"user","agent":"user"}"#).unwrap();
        store.save_message(&session.id, 3, r#"{"role":"assistant","agent":"reviewer"}"#).unwrap();

        let metrics = NullStats;
        let stats = compute_stats(&store, &session.id, &metrics);

        // message_count should reflect messages.len() from the session
        assert_eq!(stats.message_count, 4);
        // agent_count: distinct agent names (user, coder, reviewer) = 3
        assert_eq!(stats.agent_count, 3);

        std::fs::remove_file(&db_path).ok();
    }

    #[test]
    fn test_compute_stats_recent_lists_capped_at_10() {
        use archon_session::storage::SessionStore;

        let temp_dir = std::env::temp_dir();
        let db_path = temp_dir.join("test_compute_stats_capped.db");
        let store = SessionStore::open(&db_path).unwrap();

        let session = store.create_session("/tmp", None, "claude-3-5-sonnet-4b").unwrap();

        // Add 15 messages to exceed the N=10 cap
        for i in 0..15 {
            let content = format!(r#"{{"role":"assistant","agent":"agent{}", "command":"/cmd{}", "tool":"tool{}"}}"#, i, i, i);
            store.save_message(&session.id, i as u64, &content).unwrap();
        }

        let metrics = NullStats;
        let stats = compute_stats(&store, &session.id, &metrics);

        // recent_commands, recent_agents, recent_tools should each be capped at 10
        assert!(stats.recent_commands.len() <= 10);
        assert!(stats.recent_agents.len() <= 10);
        assert!(stats.recent_tools.len() <= 10);

        std::fs::remove_file(&db_path).ok();
    }

    #[test]
    fn test_compute_stats_missing_session_returns_empty() {
        use archon_session::storage::SessionStore;

        let temp_dir = std::env::temp_dir();
        let db_path = temp_dir.join("test_compute_stats_missing.db");
        let store = SessionStore::open(&db_path).unwrap();

        // Do not create any session
        let metrics = NullStats;
        let stats = compute_stats(&store, "nonexistent-session-id", &metrics);

        // Should return empty stats, not panic
        assert_eq!(stats.message_count, 0);
        assert!(stats.recent_commands.is_empty());
        assert!(stats.recent_agents.is_empty());
        assert!(stats.recent_tools.is_empty());

        std::fs::remove_file(&db_path).ok();
    }

    #[test]
    fn test_null_stats_all_zero() {
        let null_stats = NullStats;
        assert_eq!(null_stats.total_sent(), 0);
        assert_eq!(null_stats.total_drained(), 0);
        assert_eq!(null_stats.backlog_depth(), 0);
    }
}
