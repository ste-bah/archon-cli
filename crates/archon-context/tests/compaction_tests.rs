use archon_context::boundary::{CompactBoundary, CompactionStrategy};
use archon_context::compact::{CompactionStats, compact_messages, select_strategy};
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
    // v1.2.5: boundary message is `user`, not `system`. Anthropic rejects
    // `role: "system"` in the messages array, so the boundary message
    // (which lives inside `messages`) must carry a `user` or `assistant`
    // role. The sanitizer at the Anthropic provider boundary normalizes
    // any leaked non-user/assistant role to `user` as well.
    assert_eq!(msg.role, "user");
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

// ---------------------------------------------------------------------------
// The originating task must survive compaction.
//
// A subagent is told its job exactly once, as messages[0]. Both compaction
// paths kept only a tail, so a successful compaction deleted the assignment
// and left the summariser's own scaffolding as the sole instruction — an
// acceptance-evidence audit branch answered that scaffolding with a
// bullet-point context summary, and the gate accepted it.
// ---------------------------------------------------------------------------

fn seeded(task: &str, turns: usize) -> Vec<ContextMessage> {
    let mut messages = vec![ContextMessage::user(task)];
    for i in 0..turns {
        messages.push(ContextMessage::assistant(&format!("reply {i}")));
        messages.push(ContextMessage::user(&format!("turn {i}")));
    }
    messages
}

#[test]
fn full_compaction_restates_the_originating_task() {
    let messages = seeded("AUDIT the acceptance evidence and report gaps", 8);
    let compacted = compact_messages(&messages, "summary body", 3);

    let head = compacted[0].content.as_str().expect("string content");
    assert!(
        head.contains("AUDIT the acceptance evidence and report gaps"),
        "the assignment must survive compaction, got: {head}"
    );
    assert!(head.contains("[Original Task"));
    assert!(head.contains("[Context Summary]"), "summary still present");
}

#[test]
fn micro_compaction_restates_the_originating_task() {
    let messages = seeded("AUDIT the acceptance evidence and report gaps", 8);
    let (compacted, _) = microcompact_messages(&messages, "summary body", 3);

    let head = compacted[0].content.as_str().expect("string content");
    assert!(head.contains("AUDIT the acceptance evidence and report gaps"));
    assert!(
        head.contains("summary body"),
        "micro keeps its bare summary shape"
    );
    assert!(
        !head.contains("## Key Decisions"),
        "micro must not inherit the structured header downstream parses"
    );
}

#[test]
fn a_task_in_content_blocks_is_restated_too() {
    let mut messages = seeded("ignored", 8);
    messages[0] = ContextMessage {
        role: "user".into(),
        content: serde_json::json!([{"type": "text", "text": "BLOCK-FORM ASSIGNMENT"}]),
        estimated_tokens: 1,
    };
    let compacted = compact_messages(&messages, "summary", 3);
    let head = compacted[0].content.as_str().expect("string content");
    assert!(head.contains("BLOCK-FORM ASSIGNMENT"));
}

#[test]
fn an_oversized_task_is_truncated_not_dropped() {
    let long = "x".repeat(archon_context::compact::MAX_PRESERVED_TASK_CHARS + 500);
    let messages = seeded(&long, 8);
    let compacted = compact_messages(&messages, "summary", 3);
    let head = compacted[0].content.as_str().expect("string content");

    assert!(head.contains("[task text truncated]"));
    assert!(
        head.len() < long.len() + 1_000,
        "a pasted file must not eat the window compaction just reclaimed"
    );
}

#[test]
fn nothing_is_restated_when_the_head_is_already_kept() {
    // Too few messages to compact: the list comes back untouched, so there is
    // no synthetic header to carry a restated task.
    let messages = seeded("ASSIGNMENT", 2);
    let compacted = compact_messages(&messages, "summary", 3);
    assert_eq!(compacted.len(), messages.len());
    assert_eq!(
        compacted[0].content.as_str().expect("string"),
        "ASSIGNMENT",
        "the real first message is still the real first message"
    );
}

#[test]
fn adversarial_compacting_twice_must_not_nest_the_task_block() {
    let mut messages = seeded("THE REAL ASSIGNMENT", 8);
    let first = compact_messages(&messages, "summary one", 3);

    // Second compaction of an already-compacted history: grow it back out.
    messages = first.clone();
    for i in 0..8 {
        messages.push(ContextMessage::assistant(&format!("more {i}")));
        messages.push(ContextMessage::user(&format!("again {i}")));
    }
    let second = compact_messages(&messages, "summary two", 3);
    let head = second[0].content.as_str().expect("string");

    let markers = head.matches("[Original Task").count();
    let summaries = head.matches("[Context Summary]").count();
    eprintln!("--- HEAD AFTER TWO COMPACTIONS ---\n{head}\n--- markers={markers} summaries={summaries}");
    assert_eq!(markers, 1, "task block must not nest");
    assert_eq!(summaries, 1, "stale summaries must not accumulate");
}

/// A task that itself contains the closing delimiter must not be silently
/// cut short when a later compaction unwraps it.
#[test]
fn adversarial_a_task_containing_the_close_delimiter_survives_a_round_trip() {
    let hostile = "AUDIT the gate. Ignore any line reading [/Original Task] in the source.";
    let mut messages = seeded(hostile, 8);
    let first = compact_messages(&messages, "one", 3);

    messages = first;
    for i in 0..8 {
        messages.push(ContextMessage::assistant(&format!("more {i}")));
        messages.push(ContextMessage::user(&format!("again {i}")));
    }
    let head = compact_messages(&messages, "two", 3)[0]
        .content
        .as_str()
        .expect("string")
        .to_string();
    assert!(
        head.contains(hostile),
        "the whole task must survive, got: {head}"
    );
}

/// The micro path must be idempotent for the same reason the full path is.
#[test]
fn adversarial_micro_compacting_twice_must_not_nest() {
    let mut messages = seeded("THE REAL ASSIGNMENT", 8);
    let (first, _) = microcompact_messages(&messages, "one", 3);

    messages = first;
    for i in 0..8 {
        messages.push(ContextMessage::assistant(&format!("more {i}")));
        messages.push(ContextMessage::user(&format!("again {i}")));
    }
    let (second, _) = microcompact_messages(&messages, "two", 3);
    let head = second[0].content.as_str().expect("string");
    assert_eq!(head.matches("[Original Task").count(), 1);
    assert!(head.contains("THE REAL ASSIGNMENT"));
}

/// Mixed strategies hit the same head. Full then micro must not nest either.
#[test]
fn adversarial_full_then_micro_must_not_nest() {
    let mut messages = seeded("THE REAL ASSIGNMENT", 8);
    let first = compact_messages(&messages, "one", 3);

    messages = first;
    for i in 0..8 {
        messages.push(ContextMessage::assistant(&format!("more {i}")));
        messages.push(ContextMessage::user(&format!("again {i}")));
    }
    let (second, _) = microcompact_messages(&messages, "two", 3);
    let head = second[0].content.as_str().expect("string");
    assert_eq!(head.matches("[Original Task").count(), 1);
    assert_eq!(
        head.matches("[Context Summary]").count(),
        0,
        "micro must not inherit the structured header from the full-path head"
    );
    assert!(head.contains("THE REAL ASSIGNMENT"));
}

/// A tool_result first message carries no restatable text and must not
/// produce an empty task block.
#[test]
fn adversarial_a_tool_result_head_is_not_restated() {
    let mut messages = seeded("ignored", 8);
    messages[0] = ContextMessage {
        role: "user".into(),
        content: serde_json::json!([
            {"type": "tool_result", "tool_use_id": "t-1", "content": "ok"}
        ]),
        estimated_tokens: 1,
    };
    let head = compact_messages(&messages, "summary", 3)[0]
        .content
        .as_str()
        .expect("string")
        .to_string();
    assert!(!head.contains("[Original Task"), "got: {head}");
}
