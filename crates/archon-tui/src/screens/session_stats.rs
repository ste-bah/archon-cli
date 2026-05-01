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

// -------------------------------------------------------------------------
// StatsSource / NullStats — lives at module level so integration tests
// can import it.  The actual ChannelMetrics implementation (follow-up
// TECH-TUI-OBSERVABILITY) will replace this shim.
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
    store: &archon_session::storage::SessionStore,
    session_id: &str,
    _metrics: &dyn StatsSource,
) -> SessionStats {
    // Fetch session; on error return empty stats (never panic)
    let session = match store.get_session(session_id) {
        Ok(s) => s,
        Err(_) => return SessionStats::empty(),
    };

    // Load all messages for this session
    let messages = match store.load_messages(session_id) {
        Ok(msgs) => msgs,
        Err(_) => return SessionStats::empty(),
    };

    let message_count = messages.len() as u64;

    // Collect distinct agents via HashSet
    let mut agents = std::collections::HashSet::new();
    // Collect recent entries (newest-first = reverse iteration, cap at 10)
    let mut recent_commands: Vec<String> = Vec::new();
    let mut recent_agents: Vec<String> = Vec::new();
    let mut recent_tools: Vec<String> = Vec::new();

    // Accumulate token usage across all messages
    let mut total_input_tokens: u64 = 0;
    let mut total_output_tokens: u64 = 0;
    let mut total_cache_creation_input_tokens: u64 = 0;
    let mut total_cache_read_input_tokens: u64 = 0;

    // Parse messages in reverse (newest-first) to build recent lists
    for content in messages.iter().rev() {
        // Parse JSON message content
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(content) {
            // Extract agent
            if let Some(agent_val) = json.get("agent").and_then(|v| v.as_str())
                && agents.insert(agent_val.to_string())
                && recent_agents.len() < 10
            {
                recent_agents.push(agent_val.to_string());
            }

            // Extract command
            if let Some(cmd_val) = json.get("command").and_then(|v| v.as_str())
                && !cmd_val.is_empty()
                && !recent_commands.contains(&cmd_val.to_string())
            {
                recent_commands.push(cmd_val.to_string());
                if recent_commands.len() > 10 {
                    recent_commands.remove(0);
                }
            }

            // Extract tool
            if let Some(tool_val) = json.get("tool").and_then(|v| v.as_str())
                && !tool_val.is_empty()
                && !recent_tools.contains(&tool_val.to_string())
            {
                recent_tools.push(tool_val.to_string());
                if recent_tools.len() > 10 {
                    recent_tools.remove(0);
                }
            }

            // Extract token usage if present
            if let Some(token_obj) = json.get("token_usage") {
                total_input_tokens += token_obj
                    .get("input_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                total_output_tokens += token_obj
                    .get("output_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                total_cache_creation_input_tokens += token_obj
                    .get("cache_creation_input_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                total_cache_read_input_tokens += token_obj
                    .get("cache_read_input_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
            }
        }
    }

    // Compute elapsed time; clamp negative durations to zero
    let created_at = chrono::DateTime::parse_from_rfc3339(&session.created_at)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .unwrap_or_else(|_| chrono::Utc::now());
    let elapsed = chrono::Utc::now().signed_duration_since(created_at);
    let elapsed = std::time::Duration::from_secs(elapsed.num_seconds().try_into().unwrap_or(0));

    SessionStats {
        message_count,
        agent_count: agents.len() as u64,
        token_usage: TokenUsage {
            input_tokens: total_input_tokens,
            output_tokens: total_output_tokens,
            cache_creation_input_tokens: total_cache_creation_input_tokens,
            cache_read_input_tokens: total_cache_read_input_tokens,
        },
        elapsed,
        estimated_cost_usd: None, // pricing table is follow-up work
        recent_commands,
        recent_agents,
        recent_tools,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_stats_empty_session_returns_zero_counts() {
        use archon_session::storage::SessionStore;

        // Create a temporary in-memory database
        let temp_dir = std::env::temp_dir();
        let db_path = temp_dir.join("test_compute_stats_empty.db");
        let store = SessionStore::open(&db_path).unwrap();

        // Create a session with no messages
        let session = store
            .create_session("/tmp", None, "claude-3-5-sonnet-4b")
            .unwrap();
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
        let session = store
            .create_session("/tmp", None, "claude-3-5-sonnet-4b")
            .unwrap();

        // Add messages with agent metadata (stored as JSON content)
        store
            .save_message(&session.id, 0, r#"{"role":"user","agent":"user"}"#)
            .unwrap();
        store
            .save_message(&session.id, 1, r#"{"role":"assistant","agent":"coder"}"#)
            .unwrap();
        store
            .save_message(&session.id, 2, r#"{"role":"user","agent":"user"}"#)
            .unwrap();
        store
            .save_message(&session.id, 3, r#"{"role":"assistant","agent":"reviewer"}"#)
            .unwrap();

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

        let session = store
            .create_session("/tmp", None, "claude-3-5-sonnet-4b")
            .unwrap();

        // Add 15 messages to exceed the N=10 cap
        for i in 0..15 {
            let content = format!(
                r#"{{"role":"assistant","agent":"agent{}", "command":"/cmd{}", "tool":"tool{}"}}"#,
                i, i, i
            );
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
