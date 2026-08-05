use super::extraction_window_from;
use super::{MAX_EXTRACTION_MESSAGE_CHARS, MAX_EXTRACTION_MESSAGES, MAX_EXTRACTION_PROMPT_CHARS};

fn msg(role: &str, content: &str) -> serde_json::Value {
    serde_json::json!({ "role": role, "content": content })
}

/// The conversation must reach the model in the order it happened.
///
/// The previous implementation collected `.rev().take(10)` and never restored
/// the order, so every extraction read the conversation backwards -- newest
/// first, with corrections appearing before the thing they corrected.
#[test]
fn window_is_chronological() {
    let messages = vec![
        msg("user", "first"),
        msg("assistant", "second"),
        msg("user", "third"),
    ];

    let window = extraction_window_from(&messages, 0);

    assert_eq!(
        window,
        vec![
            "user: first".to_string(),
            "assistant: second".to_string(),
            "user: third".to_string(),
        ]
    );
}

/// Everything since the last extraction is examined, however many turns that is.
///
/// This is the whole point of the change: a fixed 10-message lookback silently
/// dropped anything older, and a correction dropped that way is lost for good
/// because the keyword pass already declined it.
#[test]
fn window_covers_everything_since_the_last_extraction() {
    let mut messages: Vec<serde_json::Value> = (0..12)
        .map(|i| msg("assistant", &format!("tool step {i}")))
        .collect();
    messages.insert(0, msg("user", "the correction that must not be lost"));

    let window = extraction_window_from(&messages, 0);

    assert_eq!(window.len(), 13);
    assert!(
        window[0].contains("must not be lost"),
        "a message older than ten back must still be in the window, got {:?}",
        window.first()
    );
}

/// Only messages after the previous extraction are re-examined.
#[test]
fn window_starts_after_the_previous_extraction() {
    let messages = vec![
        msg("user", "already examined"),
        msg("assistant", "also examined"),
        msg("user", "new since then"),
    ];

    let window = extraction_window_from(&messages, 2);

    assert_eq!(window, vec!["user: new since then".to_string()]);
}

/// A start index past the end must not panic.
///
/// `state.messages` is truncated by compaction and rewind, so the recorded
/// index can outlive the messages it pointed at.
#[test]
fn a_start_index_beyond_the_end_yields_an_empty_window() {
    let messages = vec![msg("user", "only one")];
    assert!(extraction_window_from(&messages, 99).is_empty());
}

/// Unbounded growth is the risk the window trades for completeness.
///
/// Tool-heavy work produces hundreds of messages between extractions, and
/// sending them all would make extraction cost more than the work it observes.
#[test]
fn window_is_capped_by_message_count_keeping_the_newest() {
    let messages: Vec<serde_json::Value> = (0..MAX_EXTRACTION_MESSAGES + 20)
        .map(|i| msg("user", &format!("m{i}")))
        .collect();

    let window = extraction_window_from(&messages, 0);

    assert_eq!(window.len(), MAX_EXTRACTION_MESSAGES);
    assert!(
        window
            .last()
            .expect("last")
            .contains(&format!("m{}", MAX_EXTRACTION_MESSAGES + 19)),
        "the newest message must survive the cap"
    );
    assert!(
        !window.iter().any(|line| line.ends_with("m0")),
        "the oldest must be the one dropped"
    );
}

/// One pasted document must not dominate the call.
#[test]
fn each_message_is_excerpted() {
    let messages = vec![msg("user", &"x".repeat(MAX_EXTRACTION_MESSAGE_CHARS * 4))];

    let window = extraction_window_from(&messages, 0);

    assert_eq!(window.len(), 1);
    assert!(
        window[0].chars().count() <= MAX_EXTRACTION_MESSAGE_CHARS + "user: ".len(),
        "a single message must not exceed its excerpt, got {}",
        window[0].chars().count()
    );
}

/// The total call stays within budget even when every message is at its cap.
#[test]
fn window_respects_the_total_character_budget() {
    let messages: Vec<serde_json::Value> = (0..MAX_EXTRACTION_MESSAGES)
        .map(|_| msg("user", &"y".repeat(MAX_EXTRACTION_MESSAGE_CHARS)))
        .collect();

    let window = extraction_window_from(&messages, 0);
    let total: usize = window.iter().map(|line| line.chars().count()).sum();

    assert!(
        total <= MAX_EXTRACTION_PROMPT_CHARS,
        "window spent {total} against a budget of {MAX_EXTRACTION_PROMPT_CHARS}"
    );
    assert!(
        !window.is_empty(),
        "the budget must still admit the most recent messages"
    );
}

#[test]
fn empty_messages_are_skipped() {
    let messages = vec![msg("user", ""), msg("assistant", "kept")];
    assert_eq!(
        extraction_window_from(&messages, 0),
        vec!["assistant: kept".to_string()]
    );
}
