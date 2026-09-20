//! The split must never hand pass 2 something it cannot use.
use super::two_pass::{
    DEFAULT_SPLIT_FRACTION, build_pass2_messages, note_for_pass2, split_for_two_pass,
};
use serde_json::{Value, json};

fn msg(role: &str, bytes: usize) -> Value {
    json!({"role": role, "content": [{"type": "text", "text": "x".repeat(bytes)}]})
}
fn tool_result(bytes: usize) -> Value {
    json!({"role": "user", "content": [
        {"type": "tool_result", "tool_use_id": "t1", "content": "y".repeat(bytes)}
    ]})
}

#[test]
fn a_short_conversation_is_not_worth_two_calls() {
    // Too few messages...
    let few: Vec<Value> = (0..4).map(|_| msg("user", 40_000)).collect();
    assert!(split_for_two_pass(&few, DEFAULT_SPLIT_FRACTION).is_none());
    // ...and enough messages but a small history: one pass is the better trade,
    // because two passes are two blocking calls and we have no background pass 1.
    let small: Vec<Value> = (0..12).map(|_| msg("user", 200)).collect();
    assert!(split_for_two_pass(&small, DEFAULT_SPLIT_FRACTION).is_none());
}

#[test]
fn the_tail_is_never_empty_and_never_the_whole_thing() {
    for count in [8usize, 12, 40, 200] {
        let messages: Vec<Value> = (0..count).map(|_| msg("user", 40_000)).collect();
        let split = split_for_two_pass(&messages, DEFAULT_SPLIT_FRACTION)
            .unwrap_or_else(|| panic!("{count} messages should split"));
        assert!(
            !split.tail.is_empty(),
            "{count}: pass 2 got no recent turns"
        );
        assert!(!split.prefix.is_empty(), "{count}: pass 1 got nothing");
        assert_eq!(split.prefix.len() + split.tail.len(), count);
    }
}

#[test]
fn the_tail_never_opens_with_an_orphaned_tool_result() {
    // A tool_result answers a tool_use in the turn before it; splitting between
    // them hands pass 2 a dangling pair that strict backends reject.
    let mut messages = vec![msg("system", 50)];
    for _ in 0..10 {
        messages.push(msg("assistant", 30_000));
        messages.push(tool_result(30_000));
    }
    for fraction in [0.5f64, 0.8, 0.9, 0.95] {
        let Some(split) = split_for_two_pass(&messages, fraction) else {
            continue;
        };
        let first = &split.tail[0];
        let opens_with_result = first["content"]
            .as_array()
            .is_some_and(|b| b.iter().any(|x| x["type"] == "tool_result"));
        assert!(
            !opens_with_result,
            "fraction {fraction}: orphaned tool_result leads the tail"
        );
    }
}

#[test]
fn pass_two_carries_the_system_turns_the_note_and_the_tail() {
    let mut messages = vec![json!({"role": "system", "content": "the standing task"})];
    for _ in 0..12 {
        messages.push(msg("user", 40_000));
    }
    messages.push(json!({"role": "user", "content": [{"type":"text","text":"the newest turn"}]}));
    let split = split_for_two_pass(&messages, DEFAULT_SPLIT_FRACTION).expect("should split");
    let built = build_pass2_messages(&split, "EARLIER NOTE TEXT");
    let rendered = serde_json::to_string(&built).unwrap();

    assert_eq!(built[0]["role"], "system");
    assert!(rendered.contains("the standing task"), "system turn lost");
    assert!(rendered.contains("EARLIER NOTE TEXT"), "pass-1 note lost");
    assert!(rendered.contains("the newest turn"), "recent tail lost");
    // The instruction the successor depends on: absorb, do not refer back.
    assert!(rendered.contains("Incorporate the earlier summary in full"));
}

#[test]
fn an_oversized_note_is_bounded_before_pass_two() {
    let huge = "n".repeat(80_000);
    let note = note_for_pass2(&huge);
    assert!(
        note.chars().count() < 13_000,
        "note not bounded: {}",
        note.chars().count()
    );
    assert!(note.contains("truncated for the pass-2 input budget"));
    // A note that already fits is passed through untouched.
    assert_eq!(note_for_pass2("  short note  "), "short note");
}
