//! Tests for model-free context pruning (#189 Phase 8).

use super::*;

fn call(id: &str, name: &str, input: serde_json::Value) -> serde_json::Value {
    serde_json::json!({"role": "assistant", "content": [{
        "type": "tool_use", "id": id, "name": name, "input": input
    }]})
}

fn result(id: &str, content: &str) -> serde_json::Value {
    serde_json::json!({"role": "user", "content": [{
        "type": "tool_result", "tool_use_id": id, "content": content, "is_error": false
    }]})
}

fn failed(id: &str, content: &str) -> serde_json::Value {
    serde_json::json!({"role": "user", "content": [{
        "type": "tool_result", "tool_use_id": id, "content": content, "is_error": true
    }]})
}

fn spilled(id: &str, content: &str, path: &str) -> serde_json::Value {
    serde_json::json!({"role": "user", "content": [{
        "type": "tool_result", "tool_use_id": id, "content": content, "is_error": false,
        crate::agent::SPILL_PATH_KEY: path
    }]})
}

fn read(path: &str) -> serde_json::Value {
    serde_json::json!({"file_path": path})
}

fn content_of(messages: &[serde_json::Value], id: &str) -> String {
    messages
        .iter()
        .filter_map(|m| m.get("content").and_then(|c| c.as_array()))
        .flatten()
        .find(|b| b.get("tool_use_id").and_then(|v| v.as_str()) == Some(id))
        .and_then(|b| b.get("content").and_then(|v| v.as_str()))
        .unwrap_or_default()
        .to_string()
}

/// The acceptance criterion: three reads of one unchanged file collapse to one,
/// and no model is involved in reaching that conclusion.
#[test]
fn three_reads_of_an_unchanged_file_collapse_to_one() {
    let body = "file contents ".repeat(400);
    let messages = vec![
        call("r1", "Read", read("/proj/src/main.rs")),
        result("r1", &body),
        call("r2", "Read", read("/proj/src/main.rs")),
        result("r2", &body),
        call("r3", "Read", read("/proj/src/main.rs")),
        result("r3", &body),
    ];

    let outcome = prune_mechanical(&messages, PruneConfig::default());

    assert!(outcome.bytes_reclaimed > 0);
    assert!(outcome.rules_fired.contains(&RULE_REPEATED_READS));
    assert!(content_of(&outcome.messages, "r1").contains("read again later"));
    assert!(content_of(&outcome.messages, "r2").contains("read again later"));
    assert_eq!(
        content_of(&outcome.messages, "r3"),
        body,
        "the newest read keeps its contents"
    );
}

/// Collapsing across a write would present stale text as current — worse than
/// spending the tokens.
#[test]
fn a_write_between_two_reads_stops_them_collapsing() {
    let before = "old contents ".repeat(400);
    let after = "new contents ".repeat(400);
    let messages = vec![
        call("r1", "Read", read("/proj/a.rs")),
        result("r1", &before),
        call("w1", "Edit", read("/proj/a.rs")),
        result("w1", "edited"),
        call("r2", "Read", read("/proj/a.rs")),
        result("r2", &after),
    ];

    let outcome = prune_mechanical(&messages, PruneConfig::default());

    assert_eq!(content_of(&outcome.messages, "r1"), before);
    assert!(!outcome.rules_fired.contains(&RULE_REPEATED_READS));
}

#[test]
fn reads_of_different_files_are_not_collapsed() {
    let body = "contents ".repeat(400);
    let messages = vec![
        call("r1", "Read", read("/proj/a.rs")),
        result("r1", &body),
        call("r2", "Read", read("/proj/b.rs")),
        result("r2", &body),
    ];

    let outcome = prune_mechanical(&messages, PruneConfig::default());

    assert!(!outcome.rules_fired.contains(&RULE_REPEATED_READS));
}

/// A write *after* the last read does not make the earlier reads disagree with
/// each other, so it must not block the collapse.
#[test]
fn a_write_after_the_final_read_does_not_block_the_collapse() {
    let body = "contents ".repeat(400);
    let messages = vec![
        call("r1", "Read", read("/proj/a.rs")),
        result("r1", &body),
        call("r2", "Read", read("/proj/a.rs")),
        result("r2", &body),
        call("w1", "Write", read("/proj/a.rs")),
        result("w1", "written"),
    ];

    let outcome = prune_mechanical(&messages, PruneConfig::default());

    assert!(content_of(&outcome.messages, "r1").contains("read again later"));
    assert_eq!(content_of(&outcome.messages, "r2"), body);
}

#[test]
fn a_spilled_result_repeated_later_becomes_a_pointer_to_its_file() {
    let body = "grep output ".repeat(3_000);
    let messages = vec![
        call("g1", "Grep", serde_json::json!({"pattern": "x"})),
        spilled("g1", &body, "/proj/.archon/spill/s/g1-Grep.txt"),
        call("g2", "Grep", serde_json::json!({"pattern": "x"})),
        result("g2", &body),
    ];

    let outcome = prune_mechanical(&messages, PruneConfig::default());

    let pruned = content_of(&outcome.messages, "g1");
    assert!(
        pruned.contains("/proj/.archon/spill/s/g1-Grep.txt"),
        "{pruned}"
    );
    assert!(outcome.rules_fired.contains(&RULE_SPILLED));
    assert_eq!(content_of(&outcome.messages, "g2"), body);
}

/// Without a locator there is nowhere to point, so the body has to stay.
#[test]
fn an_unspilled_repeat_is_left_alone() {
    let body = "output ".repeat(3_000);
    let messages = vec![
        call("g1", "Grep", serde_json::json!({"pattern": "x"})),
        result("g1", &body),
        call("g2", "Grep", serde_json::json!({"pattern": "x"})),
        result("g2", &body),
    ];

    let outcome = prune_mechanical(&messages, PruneConfig::default());

    assert!(!outcome.rules_fired.contains(&RULE_SPILLED));
    assert_eq!(content_of(&outcome.messages, "g1"), body);
}

#[test]
fn a_failure_retried_successfully_is_replaced_with_a_note() {
    let error = "compilation failed\n".repeat(500);
    let input = serde_json::json!({"command": "cargo build"});
    let messages = vec![
        call("b1", "Bash", input.clone()),
        failed("b1", &error),
        call("b2", "Bash", input),
        result("b2", "ok"),
    ];

    let outcome = prune_mechanical(&messages, PruneConfig::default());

    assert!(outcome.rules_fired.contains(&RULE_RETRIED_ERRORS));
    assert!(content_of(&outcome.messages, "b1").contains("succeeded later"));
}

/// A failure that was never retried is the whole diagnostic record. Removing it
/// would delete the reason the turn is in the state it is in.
#[test]
fn a_failure_that_was_never_retried_is_kept() {
    let error = "compilation failed\n".repeat(500);
    let messages = vec![
        call("b1", "Bash", serde_json::json!({"command": "cargo build"})),
        failed("b1", &error),
    ];

    let outcome = prune_mechanical(&messages, PruneConfig::default());

    assert_eq!(content_of(&outcome.messages, "b1"), error);
    assert!(!outcome.reclaimed_anything());
}

/// A different command succeeding says nothing about this one.
#[test]
fn a_different_call_succeeding_does_not_excuse_the_failure() {
    let error = "failed\n".repeat(500);
    let messages = vec![
        call("b1", "Bash", serde_json::json!({"command": "cargo build"})),
        failed("b1", &error),
        call("b2", "Bash", serde_json::json!({"command": "cargo fmt"})),
        result("b2", "ok"),
    ];

    let outcome = prune_mechanical(&messages, PruneConfig::default());

    assert_eq!(content_of(&outcome.messages, "b1"), error);
}

/// The invariant the whole module is shaped around. An assistant `tool_use`
/// with no matching `tool_result` is a provider-level error, so no rule may
/// remove a block — only shorten one.
#[test]
fn every_tool_use_still_has_its_result_afterwards() {
    let body = "contents ".repeat(400);
    let messages = vec![
        call("r1", "Read", read("/proj/a.rs")),
        result("r1", &body),
        call("r2", "Read", read("/proj/a.rs")),
        result("r2", &body),
    ];

    let outcome = prune_mechanical(&messages, PruneConfig::default());

    let ids: Vec<&str> = outcome
        .messages
        .iter()
        .filter_map(|m| m.get("content").and_then(|c| c.as_array()))
        .flatten()
        .filter(|b| b.get("type").and_then(|v| v.as_str()) == Some("tool_result"))
        .filter_map(|b| b.get("tool_use_id").and_then(|v| v.as_str()))
        .collect();

    assert_eq!(ids, vec!["r1", "r2"]);
    assert_eq!(outcome.messages.len(), messages.len());
}

#[test]
fn each_rule_can_be_disabled_on_its_own() {
    let body = "contents ".repeat(400);
    let messages = vec![
        call("r1", "Read", read("/proj/a.rs")),
        result("r1", &body),
        call("r2", "Read", read("/proj/a.rs")),
        result("r2", &body),
    ];

    let without_reads = PruneConfig {
        repeated_reads: false,
        ..PruneConfig::default()
    };

    assert!(!prune_mechanical(&messages, without_reads).reclaimed_anything());
    assert!(prune_mechanical(&messages, PruneConfig::default()).reclaimed_anything());
}

#[test]
fn a_disabled_pass_returns_the_history_untouched() {
    let body = "contents ".repeat(400);
    let messages = vec![
        call("r1", "Read", read("/proj/a.rs")),
        result("r1", &body),
        call("r2", "Read", read("/proj/a.rs")),
        result("r2", &body),
    ];

    let disabled = PruneConfig {
        enabled: false,
        ..PruneConfig::default()
    };

    let outcome = prune_mechanical(&messages, disabled);
    assert_eq!(outcome.messages, messages);
    assert!(!outcome.reclaimed_anything());
}

/// A rule that made a result longer would report a saving while costing
/// tokens. Short bodies are left alone.
#[test]
fn a_replacement_longer_than_the_original_is_declined() {
    let messages = vec![
        call("r1", "Read", read("/a")),
        result("r1", "hi"),
        call("r2", "Read", read("/a")),
        result("r2", "hi"),
    ];

    let outcome = prune_mechanical(&messages, PruneConfig::default());

    assert_eq!(content_of(&outcome.messages, "r1"), "hi");
    assert!(!outcome.reclaimed_anything());
}

#[test]
fn an_empty_conversation_prunes_to_nothing() {
    let outcome = prune_mechanical(&[], PruneConfig::default());
    assert!(outcome.messages.is_empty());
    assert!(!outcome.reclaimed_anything());
    assert!(outcome.rules_fired.is_empty());
}

/// Ordinary prose messages carry no tool results and must survive verbatim.
#[test]
fn plain_messages_are_untouched() {
    let messages = vec![
        serde_json::json!({"role": "user", "content": "hello"}),
        serde_json::json!({"role": "assistant", "content": [{"type": "text", "text": "hi"}]}),
    ];

    assert_eq!(
        prune_mechanical(&messages, PruneConfig::default()).messages,
        messages
    );
}

/// Key order is load-bearing: cache markers are position-sensitive, so a
/// rewritten block must serialize exactly as an in-place overwrite would.
#[test]
fn a_rewritten_block_preserves_key_order() {
    let body = "contents ".repeat(400);
    let messages = vec![
        call("r1", "Read", read("/a")),
        serde_json::json!({"role": "user", "content": [{
            "type": "tool_result", "tool_use_id": "r1", "content": body,
            "is_error": false, "trailing": "kept"
        }]}),
        call("r2", "Read", read("/a")),
        result("r2", &body),
    ];

    let outcome = prune_mechanical(&messages, PruneConfig::default());

    let mut overwritten = messages.clone();
    overwritten[1]["content"][0]["content"] = outcome.messages[1]["content"][0]["content"].clone();
    assert_eq!(
        serde_json::to_string(&outcome.messages[1]).expect("serialize pruned"),
        serde_json::to_string(&overwritten[1]).expect("serialize overwrite")
    );
}

#[test]
fn reclaimed_bytes_match_the_shrinkage() {
    let body = "contents ".repeat(400);
    let messages = vec![
        call("r1", "Read", read("/a")),
        result("r1", &body),
        call("r2", "Read", read("/a")),
        result("r2", &body),
    ];

    let outcome = prune_mechanical(&messages, PruneConfig::default());

    let before = content_of(&messages, "r1").len();
    let after = content_of(&outcome.messages, "r1").len();
    assert_eq!(outcome.bytes_reclaimed, before - after);
}
