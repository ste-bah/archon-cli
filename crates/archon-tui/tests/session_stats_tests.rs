//! Tests for SessionStats struct.
//!
//! GATE 1: tests-written-first — These tests were created before implementation.

use archon_llm::TokenUsage;
use std::time::Duration;

#[test]
fn session_stats_empty_returns_is_empty_true() {
    let stats = archon_tui::screens::session_stats::SessionStats::empty();
    assert!(
        stats.is_empty(),
        "SessionStats::empty() should return is_empty() == true"
    );
}

#[test]
fn session_stats_default_returns_is_empty_true() {
    let stats = archon_tui::screens::session_stats::SessionStats::default();
    assert!(
        stats.is_empty(),
        "SessionStats::default() should return is_empty() == true"
    );
}

#[test]
fn session_stats_empty_has_zero_counts() {
    let stats = archon_tui::screens::session_stats::SessionStats::empty();
    assert_eq!(
        stats.message_count, 0,
        "empty() should have message_count == 0"
    );
    assert_eq!(stats.agent_count, 0, "empty() should have agent_count == 0");
}

#[test]
fn session_stats_with_nonzero_counts_is_not_empty() {
    use archon_tui::screens::session_stats::SessionStats;

    let stats = SessionStats {
        message_count: 5,
        agent_count: 2,
        token_usage: TokenUsage::default(),
        elapsed: Duration::from_secs(10),
        estimated_cost_usd: Some(0.05),
        recent_commands: vec!["ls".to_string()],
        recent_agents: vec!["coder".to_string()],
        recent_tools: vec!["Read".to_string()],
    };

    assert!(
        !stats.is_empty(),
        "SessionStats with non-zero counts should return is_empty() == false"
    );
}

#[test]
fn token_usage_field_is_archon_llm_token_usage_type() {
    use archon_tui::screens::session_stats::SessionStats;

    // This test verifies the token_usage field is of type archon_llm::TokenUsage
    // If TokenUsage doesn't exist or has wrong structure, this will fail to compile
    let usage = TokenUsage::default();
    let stats = SessionStats {
        message_count: 0,
        agent_count: 0,
        token_usage: usage,
        elapsed: Duration::ZERO,
        estimated_cost_usd: None,
        recent_commands: vec![],
        recent_agents: vec![],
        recent_tools: vec![],
    };

    // Verify the token_usage field can be read
    let _ = stats.token_usage;
}

#[test]
fn elapsed_field_is_std_time_duration_type() {
    use archon_tui::screens::session_stats::SessionStats;

    // This test verifies the elapsed field is of type std::time::Duration
    let stats = SessionStats {
        message_count: 0,
        agent_count: 0,
        token_usage: TokenUsage::default(),
        elapsed: Duration::from_secs(60),
        estimated_cost_usd: None,
        recent_commands: vec![],
        recent_agents: vec![],
        recent_tools: vec![],
    };

    // Verify the elapsed field can be read and has expected value
    assert_eq!(stats.elapsed, Duration::from_secs(60));
}

#[test]
fn estimated_cost_usd_is_option_f64() {
    use archon_tui::screens::session_stats::SessionStats;

    // Test Some case
    let stats_with_cost = SessionStats {
        message_count: 0,
        agent_count: 0,
        token_usage: TokenUsage::default(),
        elapsed: Duration::ZERO,
        estimated_cost_usd: Some(1.25),
        recent_commands: vec![],
        recent_agents: vec![],
        recent_tools: vec![],
    };
    assert!(stats_with_cost.estimated_cost_usd.is_some());
    assert_eq!(stats_with_cost.estimated_cost_usd.unwrap(), 1.25);

    // Test None case
    let stats_without_cost = SessionStats {
        message_count: 0,
        agent_count: 0,
        token_usage: TokenUsage::default(),
        elapsed: Duration::ZERO,
        estimated_cost_usd: None,
        recent_commands: vec![],
        recent_agents: vec![],
        recent_tools: vec![],
    };
    assert!(stats_without_cost.estimated_cost_usd.is_none());
}

#[test]
fn recent_commands_agents_tools_are_vec_string() {
    use archon_tui::screens::session_stats::SessionStats;

    let stats = SessionStats {
        message_count: 0,
        agent_count: 0,
        token_usage: TokenUsage::default(),
        elapsed: Duration::ZERO,
        estimated_cost_usd: None,
        recent_commands: vec!["ls".to_string(), "pwd".to_string()],
        recent_agents: vec!["coder".to_string(), "reviewer".to_string()],
        recent_tools: vec!["Read".to_string(), "Write".to_string()],
    };

    assert_eq!(stats.recent_commands.len(), 2);
    assert_eq!(stats.recent_agents.len(), 2);
    assert_eq!(stats.recent_tools.len(), 2);

    // Verify they are actually Strings
    assert_eq!(stats.recent_commands[0], "ls");
    assert_eq!(stats.recent_agents[0], "coder");
    assert_eq!(stats.recent_tools[0], "Read");
}
