//! Tests for the cassette key (#189 Phase 5).
//!
//! Two failure modes, opposite and both fatal. Under-normalise and no replay
//! ever hits, so the mechanism is useless. Over-normalise and two different
//! requests share a cassette, so a test passes against the wrong recording —
//! which is worse, because it looks like it worked.

use super::*;

use crate::provider::shared_tools;

fn request() -> LlmRequest {
    LlmRequest {
        model: "claude-sonnet-4-6".into(),
        messages: vec![serde_json::json!({"role": "user", "content": "hello"})],
        ..Default::default()
    }
}

#[test]
fn the_same_request_hashes_the_same_way_twice() {
    assert_eq!(digest(&request()), digest(&request()));
}

#[test]
fn a_different_question_hashes_differently() {
    let mut other = request();
    other.messages = vec![serde_json::json!({"role": "user", "content": "goodbye"})];

    assert_ne!(digest(&request()), digest(&other));
}

#[test]
fn the_model_is_part_of_the_key() {
    let mut other = request();
    other.model = "claude-opus-4-8".into();

    assert_ne!(digest(&request()), digest(&other));
}

#[test]
fn the_system_prompt_is_part_of_the_key() {
    let mut other = request();
    other.system = vec![serde_json::json!({"type": "text", "text": "be terse"})];

    assert_ne!(digest(&request()), digest(&other));
}

#[test]
fn the_tool_schemas_are_part_of_the_key() {
    let mut other = request();
    other.tools = shared_tools(vec![serde_json::json!({"name": "Read"})]);

    assert_ne!(digest(&request()), digest(&other));
}

/// Tool ids are generated per turn. If they counted, a cassette would be
/// unreplayable the moment it was recorded.
#[test]
fn tool_ids_do_not_count() {
    let mut first = request();
    first.messages = vec![serde_json::json!({
        "role": "assistant",
        "content": [{"type": "tool_use", "id": "toolu_01AAA", "name": "Read", "input": {}}]
    })];
    let mut second = first.clone();
    second.messages = vec![serde_json::json!({
        "role": "assistant",
        "content": [{"type": "tool_use", "id": "toolu_99ZZZ", "name": "Read", "input": {}}]
    })];

    assert_eq!(digest(&first), digest(&second));
}

/// The tool *called* still counts, or two different actions would share a
/// recording — the over-normalisation failure.
#[test]
fn the_tool_being_called_still_counts() {
    let mut first = request();
    first.messages = vec![serde_json::json!({
        "role": "assistant",
        "content": [{"type": "tool_use", "id": "toolu_1", "name": "Read", "input": {}}]
    })];
    let mut second = first.clone();
    second.messages = vec![serde_json::json!({
        "role": "assistant",
        "content": [{"type": "tool_use", "id": "toolu_1", "name": "Write", "input": {}}]
    })];

    assert_ne!(digest(&first), digest(&second));
}

/// Cache breakpoints move as context grows and are instructions to the
/// provider's cache, not part of the question.
#[test]
fn cache_breakpoints_do_not_count() {
    let mut marked = request();
    marked.system = vec![serde_json::json!({
        "type": "text",
        "text": "system",
        "cache_control": {"type": "ephemeral"}
    })];
    let mut plain = request();
    plain.system = vec![serde_json::json!({"type": "text", "text": "system"})];

    assert_eq!(digest(&marked), digest(&plain));
}

/// The spill locator (#189 Phase 1) is a path under someone's home directory.
/// A cassette that depended on it would replay only on the machine that made it.
#[test]
fn the_spill_locator_does_not_count() {
    let mut spilled = request();
    spilled.messages = vec![serde_json::json!({
        "role": "user",
        "content": [{
            "type": "tool_result",
            "tool_use_id": "toolu_1",
            "content": "output",
            "archon_spill": {"path": "/home/someone/.archon/spill/abc", "bytes": 900_000}
        }]
    })];
    let mut plain = request();
    plain.messages = vec![serde_json::json!({
        "role": "user",
        "content": [{"type": "tool_result", "tool_use_id": "toolu_1", "content": "output"}]
    })];

    assert_eq!(digest(&spilled), digest(&plain));
}

/// The `archon_runtime` envelope rides under `extra` and carries a fresh UUID
/// per run. Recording the same subagent twice and diffing the canonical forms
/// showed these were the *only* fields that differed — until they came out,
/// nothing ever hit.
#[test]
fn the_runtime_envelope_ids_do_not_count() {
    let envelope = |run: &str| {
        serde_json::json!({
            "archon_runtime": {
                "origin": "subagent", "round": 0, "turn": 0,
                "run_id": run, "session_id": run,
            }
        })
    };
    let mut first = request();
    first.extra = envelope("80208d2a-c94a-441e-971b-e934d25f26d2");
    let mut second = request();
    second.extra = envelope("a2238172-4753-4592-9e03-a5653a18a97a");

    assert_eq!(digest(&first), digest(&second));
}

/// The counters in that same envelope are what tell turn one from turn two, so
/// stripping them would collapse a whole conversation onto one cassette.
#[test]
fn the_runtime_envelope_turn_counters_still_count() {
    let at_turn = |turn: u32| {
        serde_json::json!({
            "archon_runtime": { "round": turn, "turn": turn, "run_id": "same" }
        })
    };
    let mut first = request();
    first.extra = at_turn(0);
    let mut second = request();
    second.extra = at_turn(1);

    assert_ne!(digest(&first), digest(&second));
}

/// A tracing marker no provider reads must not split a cassette in two.
#[test]
fn the_request_origin_marker_does_not_count() {
    let mut main = request();
    main.request_origin = Some("main_session".into());
    let mut sub = request();
    sub.request_origin = Some("subagent".into());

    assert_eq!(digest(&main), digest(&sub));
}

/// The Codex reasoning blob comes back different on every turn even when the
/// conversation has not moved.
#[test]
fn the_encrypted_reasoning_blob_does_not_count() {
    let mut carrying = request();
    carrying.reasoning_encrypted = Some("opaque-blob-a".into());
    let mut other = request();
    other.reasoning_encrypted = Some("opaque-blob-b".into());

    assert_eq!(digest(&carrying), digest(&other));
}

/// Thinking budget and effort change the answer, so they change the key.
#[test]
fn the_thinking_and_effort_settings_count() {
    let mut thinking = request();
    thinking.thinking = Some(serde_json::json!({"type": "enabled", "budget_tokens": 8192}));
    assert_ne!(digest(&request()), digest(&thinking));

    let mut effort = request();
    effort.effort = Some("low".into());
    assert_ne!(digest(&request()), digest(&effort));
}

/// Key order in a JSON object is not meaningful, and two serialisations of the
/// same request must not disagree about it.
#[test]
fn key_order_does_not_change_the_key() {
    let mut one = request();
    one.extra = serde_json::json!({"alpha": 1, "beta": 2});
    let mut two = request();
    two.extra = serde_json::json!({"beta": 2, "alpha": 1});

    assert_eq!(digest(&one), digest(&two));
}

#[test]
fn a_digest_is_hex_and_fits_a_filename() {
    let key = digest(&request());

    assert_eq!(key.len(), 32);
    assert!(key.chars().all(|c| c.is_ascii_hexdigit()), "{key}");
}

/// The canonical form is what a miss is diagnosed with, so it has to actually
/// contain the request rather than a hash of it.
#[test]
fn the_canonical_form_shows_the_request() {
    let canonical = canonical_json(&request());

    assert!(canonical.contains("hello"), "{canonical}");
    assert!(canonical.contains("claude-sonnet-4-6"), "{canonical}");
}
