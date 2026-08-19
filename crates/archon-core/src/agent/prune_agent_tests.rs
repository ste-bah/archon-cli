//! Tests for the agent-level mechanical prune step (#189 Phase 8).

use crate::agent::prune::{PruneOutcome, prune_mechanical};
use crate::config::PruneConfig;

fn call(id: &str, name: &str, path: &str) -> serde_json::Value {
    serde_json::json!({"role": "assistant", "content": [{
        "type": "tool_use", "id": id, "name": name, "input": {"file_path": path}
    }]})
}

fn result(id: &str, content: &str) -> serde_json::Value {
    serde_json::json!({"role": "user", "content": [{
        "type": "tool_result", "tool_use_id": id, "content": content, "is_error": false
    }]})
}

/// Three reads of one unchanged file, which is the case the acceptance
/// criterion names. The saving has to be large enough to matter, or the skip
/// decision above it is deciding on noise.
fn repeated_reads() -> Vec<serde_json::Value> {
    let body = "file contents ".repeat(4_000);
    vec![
        call("r1", "Read", "/proj/a.rs"),
        result("r1", &body),
        call("r2", "Read", "/proj/a.rs"),
        result("r2", &body),
        call("r3", "Read", "/proj/a.rs"),
        result("r3", &body),
    ]
}

fn tokens(messages: &[serde_json::Value]) -> u64 {
    crate::agent::autocompact::trigger_tokens(messages)
}

#[test]
fn pruning_shrinks_the_estimate_it_is_judged_against() {
    let before = repeated_reads();
    let outcome: PruneOutcome = prune_mechanical(&before, PruneConfig::default());

    assert!(outcome.reclaimed_anything());
    assert!(
        tokens(&outcome.messages) < tokens(&before),
        "the post-prune estimate must actually be smaller: {} vs {}",
        tokens(&outcome.messages),
        tokens(&before)
    );
}

/// The skip decision is a comparison against the threshold, and it must be
/// made on the rewritten history — the provider's number described the
/// messages as they were and is stale in exactly the direction that matters.
#[test]
fn a_large_saving_can_bring_the_estimate_under_a_threshold_it_was_over() {
    let before = repeated_reads();
    let window = tokens(&before);
    let threshold = 0.8f32;

    assert!(
        (tokens(&before) as f32 / window as f32) >= threshold,
        "fixture must start over threshold"
    );

    let after = prune_mechanical(&before, PruneConfig::default()).messages;

    assert!(
        (tokens(&after) as f32 / window as f32) < threshold,
        "pruning alone should clear the trigger here"
    );
}

/// A conversation with nothing mechanically removable must fall through to the
/// model rather than report a saving it did not make.
#[test]
fn an_unprunable_conversation_reclaims_nothing() {
    let messages = vec![
        serde_json::json!({"role": "user", "content": "explain this"}),
        serde_json::json!({"role": "assistant", "content": [{"type": "text", "text": "sure"}]}),
    ];

    let outcome = prune_mechanical(&messages, PruneConfig::default());

    assert!(!outcome.reclaimed_anything());
    assert_eq!(outcome.messages, messages);
}

/// Config off means the pass is inert, so the model path is reached unchanged.
#[test]
fn a_disabled_pass_reclaims_nothing_even_when_there_is_work_to_do() {
    let messages = repeated_reads();
    let disabled = PruneConfig {
        enabled: false,
        ..PruneConfig::default()
    };

    assert!(!prune_mechanical(&messages, disabled).reclaimed_anything());
    assert!(prune_mechanical(&messages, PruneConfig::default()).reclaimed_anything());
}
