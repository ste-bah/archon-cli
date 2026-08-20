//! The originating task must survive compaction.
//!
//! A subagent is told its job exactly once, as messages[0]. Both compaction
//! paths kept only a tail, so a successful compaction deleted the assignment
//! and left the summariser's own scaffolding as the sole instruction — an
//! acceptance-evidence audit branch answered that scaffolding with a
//! bullet-point context summary, and the gate accepted it.
//!
//! Split from `compaction_tests.rs` for the 500-line ceiling.

use archon_context::compact::compact_messages;
use archon_context::messages::ContextMessage;
use archon_context::microcompact::microcompact_messages;

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
    let long = "x".repeat(archon_context::compact_task_block::MAX_PRESERVED_TASK_CHARS + 500);
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
    eprintln!(
        "--- HEAD AFTER TWO COMPACTIONS ---\n{head}\n--- markers={markers} summaries={summaries}"
    );
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
