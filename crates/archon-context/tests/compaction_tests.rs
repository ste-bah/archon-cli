use archon_context::boundary::{CompactBoundary, CompactionStrategy};
use archon_context::compact::{CompactionStats, select_strategy};
use archon_context::messages::ContextMessage;
use archon_context::microcompact::microcompact_messages;
use archon_context::snip::{count_turns, snip_messages};

// ---------------------------------------------------------------------------
// Strategy selection
// ---------------------------------------------------------------------------

#[test]
fn select_strategy_below_60() {
    assert_eq!(select_strategy(0.5), None);
}

#[test]
fn select_strategy_micro_at_60() {
    assert_eq!(select_strategy(0.6), Some(CompactionStrategy::Micro));
}

#[test]
fn select_strategy_auto_at_80() {
    assert_eq!(select_strategy(0.8), Some(CompactionStrategy::Auto));
}

#[test]
fn select_strategy_snip_at_90() {
    assert_eq!(select_strategy(0.9), Some(CompactionStrategy::Snip));
}

// ---------------------------------------------------------------------------
// Microcompact
// ---------------------------------------------------------------------------

#[test]
fn microcompact_preserves_recent() {
    // 10 messages = 5 user + 5 assistant
    let messages: Vec<ContextMessage> = (0..10)
        .map(|i| {
            if i % 2 == 0 {
                ContextMessage::user(&format!("user msg {i}"))
            } else {
                ContextMessage::assistant(&format!("assistant msg {i}"))
            }
        })
        .collect();

    let (result, _boundary) = microcompact_messages(&messages, "Summary of old stuff", 3);

    // Recent 3 turns = last 6 messages must be intact
    let recent = &result[result.len() - 6..];
    for (idx, msg) in recent.iter().enumerate() {
        let original = &messages[messages.len() - 6 + idx];
        assert_eq!(
            msg.content.as_str().unwrap(),
            original.content.as_str().unwrap(),
            "recent message {idx} should be preserved verbatim"
        );
    }
}

#[test]
fn microcompact_summarizes_oldest() {
    let messages: Vec<ContextMessage> = (0..10)
        .map(|i| {
            if i % 2 == 0 {
                ContextMessage::user(&format!("user msg {i}"))
            } else {
                ContextMessage::assistant(&format!("assistant msg {i}"))
            }
        })
        .collect();

    let (result, _boundary) = microcompact_messages(&messages, "Summary of old stuff", 3);

    // First message should be the summary
    let first_content = result[0].content.as_str().unwrap();
    assert!(
        first_content.contains("Summary of old stuff"),
        "first message should contain the summary text"
    );
}

#[test]
fn microcompact_boundary_inserted() {
    let messages: Vec<ContextMessage> = (0..10)
        .map(|i| {
            if i % 2 == 0 {
                ContextMessage::user(&format!("user msg {i}"))
            } else {
                ContextMessage::assistant(&format!("assistant msg {i}"))
            }
        })
        .collect();

    let (result, boundary) = microcompact_messages(&messages, "Summary of old stuff", 3);

    // Boundary should be present in the result as a system-like message
    assert_eq!(boundary.strategy, CompactionStrategy::Micro);

    // There should be a boundary message between summary and recent messages
    let boundary_msg = &result[1];
    let content = boundary_msg.content.as_str().unwrap();
    assert!(
        content.contains("Micro"),
        "boundary message should mention the strategy"
    );
}

#[test]
fn microcompact_too_few_messages() {
    let messages = vec![
        ContextMessage::user("hello"),
        ContextMessage::assistant("hi"),
        ContextMessage::user("how"),
        ContextMessage::assistant("fine"),
    ];

    let (result, boundary) = microcompact_messages(&messages, "Summary", 3);

    // Not enough to compact — should return unchanged
    assert_eq!(result.len(), messages.len());
    assert_eq!(boundary.tokens_removed, 0);
}

// ---------------------------------------------------------------------------
// Snip
// ---------------------------------------------------------------------------

#[test]
fn snip_removes_exact_range() {
    // 4 turns: user+assistant pairs
    let messages = vec![
        ContextMessage::user("turn 1 user"),
        ContextMessage::assistant("turn 1 assistant"),
        ContextMessage::user("turn 2 user"),
        ContextMessage::assistant("turn 2 assistant"),
        ContextMessage::user("turn 3 user"),
        ContextMessage::assistant("turn 3 assistant"),
        ContextMessage::user("turn 4 user"),
        ContextMessage::assistant("turn 4 assistant"),
    ];

    let (result, boundary) = snip_messages(&messages, 2, 3).unwrap();

    // Turns 2 and 3 removed (4 messages), turns 1 and 4 remain (4 messages) + 1 boundary
    assert_eq!(boundary.strategy, CompactionStrategy::Snip);

    // Turn 1 present
    assert!(result.iter().any(|m| {
        m.content
            .as_str()
            .is_some_and(|s| s.contains("turn 1 user"))
    }));
    // Turn 4 present
    assert!(result.iter().any(|m| {
        m.content
            .as_str()
            .is_some_and(|s| s.contains("turn 4 user"))
    }));
    // Turn 2 absent
    assert!(!result.iter().any(|m| {
        m.content
            .as_str()
            .is_some_and(|s| s.contains("turn 2 user"))
    }));
    // Turn 3 absent
    assert!(!result.iter().any(|m| {
        m.content
            .as_str()
            .is_some_and(|s| s.contains("turn 3 user"))
    }));
}

#[test]
fn snip_invalid_range_errors() {
    let messages = vec![
        ContextMessage::user("u1"),
        ContextMessage::assistant("a1"),
        ContextMessage::user("u2"),
        ContextMessage::assistant("a2"),
    ];

    let result = snip_messages(&messages, 3, 1);
    assert!(result.is_err(), "start > end should error");
}

#[test]
fn snip_out_of_bounds_errors() {
    let messages = vec![ContextMessage::user("u1"), ContextMessage::assistant("a1")];

    let result = snip_messages(&messages, 1, 5);
    assert!(result.is_err(), "range exceeding turn count should error");
}

#[test]
fn snip_preserves_surrounding() {
    let messages = vec![
        ContextMessage::user("before user"),
        ContextMessage::assistant("before assistant"),
        ContextMessage::user("middle user"),
        ContextMessage::assistant("middle assistant"),
        ContextMessage::user("after user"),
        ContextMessage::assistant("after assistant"),
    ];

    let (result, _) = snip_messages(&messages, 2, 2).unwrap();

    // Before and after should be present
    assert!(result.iter().any(|m| {
        m.content
            .as_str()
            .is_some_and(|s| s.contains("before user"))
    }));
    assert!(
        result
            .iter()
            .any(|m| { m.content.as_str().is_some_and(|s| s.contains("after user")) })
    );
    // Middle removed
    assert!(!result.iter().any(|m| {
        m.content
            .as_str()
            .is_some_and(|s| s.contains("middle user"))
    }));
}

#[test]
fn snip_removes_complete_turns() {
    // Turn 2 has a tool call chain: user -> assistant (tool_use) -> user (tool_result) -> assistant
    let messages = vec![
        ContextMessage::user("turn 1 user"),
        ContextMessage::assistant("turn 1 assistant"),
        ContextMessage::user("turn 2 user"),
        ContextMessage::assistant("turn 2 tool call"),
        ContextMessage::user("turn 2 tool result"), // not a new turn — no new "user" intent
        ContextMessage::assistant("turn 2 final"),
        ContextMessage::user("turn 3 user"),
        ContextMessage::assistant("turn 3 assistant"),
    ];

    // This is tricky: the tool result looks like a user message but is part of turn 2.
    // count_turns sees 4 user messages = 4 turns.
    // So we snip turns 2-3 which covers messages at indices 2,3,4,5.
    let turn_count = count_turns(&messages);
    // 4 user messages = 4 turns
    assert_eq!(turn_count, 4);

    let (result, _) = snip_messages(&messages, 2, 3).unwrap();

    // Turn 1 and turn 4 remain
    assert!(result.iter().any(|m| {
        m.content
            .as_str()
            .is_some_and(|s| s.contains("turn 1 user"))
    }));
    assert!(result.iter().any(|m| {
        m.content
            .as_str()
            .is_some_and(|s| s.contains("turn 3 assistant"))
    }));
    // Turn 2 removed entirely (including tool chain)
    assert!(
        !result
            .iter()
            .any(|m| m.content.as_str().is_some_and(|s| s.contains("turn 2")))
    );
}

#[test]
fn count_turns_correct() {
    let messages = vec![
        ContextMessage::user("u1"),
        ContextMessage::assistant("a1"),
        ContextMessage::user("u2"),
        ContextMessage::assistant("a2"),
        ContextMessage::user("u3"),
        ContextMessage::assistant("a3"),
        ContextMessage::user("u4"),
        ContextMessage::assistant("a4"),
    ];

    assert_eq!(count_turns(&messages), 4);
}

// ---------------------------------------------------------------------------
// Boundary
// ---------------------------------------------------------------------------

#[test]
fn boundary_to_message_format() {
    let boundary = CompactBoundary {
        summary: "Removed old messages".into(),
        tokens_removed: 5000,
        tokens_remaining: 15000,
        strategy: CompactionStrategy::Micro,
        timestamp: chrono::Utc::now(),
    };

    let msg = boundary.to_message();
    assert_eq!(msg.role, "system");
    let content = msg.content.as_str().unwrap();
    assert!(!content.is_empty());
}

#[test]
fn boundary_shows_strategy() {
    for (strategy, label) in [
        (CompactionStrategy::Micro, "Micro"),
        (CompactionStrategy::Auto, "Auto"),
        (CompactionStrategy::Snip, "Snip"),
    ] {
        let boundary = CompactBoundary {
            summary: "test".into(),
            tokens_removed: 100,
            tokens_remaining: 900,
            strategy,
            timestamp: chrono::Utc::now(),
        };
        let msg = boundary.to_message();
        let content = msg.content.as_str().unwrap();
        assert!(
            content.contains(label),
            "boundary message should contain strategy label '{label}'"
        );
    }
}

#[test]
fn boundary_shows_tokens() {
    let boundary = CompactBoundary {
        summary: "test".into(),
        tokens_removed: 4200,
        tokens_remaining: 10000,
        strategy: CompactionStrategy::Auto,
        timestamp: chrono::Utc::now(),
    };
    let msg = boundary.to_message();
    let content = msg.content.as_str().unwrap();
    assert!(
        content.contains("4200"),
        "boundary message should show tokens_removed count"
    );
}

// ---------------------------------------------------------------------------
// Stats
// ---------------------------------------------------------------------------

#[test]
fn compaction_stats_ratio() {
    let stats = CompactionStats {
        strategy: CompactionStrategy::Auto,
        tokens_before: 1000,
        tokens_after: 500,
        messages_removed: 5,
        ratio: 500.0 / 1000.0,
    };
    assert!((stats.ratio - 0.5).abs() < f64::EPSILON);
}
