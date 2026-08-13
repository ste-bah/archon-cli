//! Claude-on-Vertex must carry `cache_control` on system blocks to the wire.
//!
//! Vertex serves the Anthropic Messages API, so the marker archon writes is
//! already the right shape. The bug was one level down: `build_claude_body`
//! joined every system block into a single string, so the marker was thrown
//! away before the request was serialised. Nothing failed — the request went
//! out, was billed in full, and `cache_strategy` still claimed Vertex cached.
//!
//! These tests pin both halves: the marker survives when present, and the
//! long-standing string form is kept when it is not.

use archon_llm::provider::LlmRequest;
use archon_llm::providers::VertexProvider;

fn text_block(text: &str) -> serde_json::Value {
    serde_json::json!({ "type": "text", "text": text })
}

fn cached_block(text: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "text",
        "text": text,
        "cache_control": { "type": "ephemeral" }
    })
}

#[test]
fn system_without_markers_stays_a_joined_string() {
    let system = vec![text_block("stable instructions"), text_block("tool notes")];

    let rendered = VertexProvider::build_system_field(&system).expect("system field is emitted");

    assert_eq!(
        rendered,
        serde_json::Value::String("stable instructions\ntool notes".into()),
        "requests that are not caching must keep the exact body Vertex has \
         always received"
    );
}

#[test]
fn empty_system_is_omitted() {
    assert!(VertexProvider::build_system_field(&[]).is_none());
}

#[test]
fn cache_control_marker_forces_the_array_form_and_survives() {
    let system = vec![
        cached_block("stable prefix worth caching"),
        text_block("volatile per-turn reminder"),
    ];

    let rendered = VertexProvider::build_system_field(&system).expect("system field is emitted");
    let blocks = rendered
        .as_array()
        .expect("a marked system prompt must serialise as an array, not a string");

    assert_eq!(blocks.len(), 2);
    assert_eq!(
        blocks[0].get("cache_control"),
        Some(&serde_json::json!({ "type": "ephemeral" })),
        "the breakpoint archon placed was dropped on the way to the wire"
    );
    assert_eq!(
        blocks[0].get("text").and_then(|t| t.as_str()),
        Some("stable prefix worth caching")
    );
    // The volatile tail stays *behind* the breakpoint — that is the whole point
    // of the stable-head placement.
    assert!(blocks[1].get("cache_control").is_none());
}

#[test]
fn blocks_missing_the_type_discriminant_are_repaired() {
    let system = vec![
        serde_json::json!({ "text": "prefix", "cache_control": { "type": "ephemeral" } }),
        serde_json::json!({ "text": "tail" }),
    ];

    let rendered = VertexProvider::build_system_field(&system).expect("system field is emitted");
    let blocks = rendered.as_array().expect("array form");

    for block in blocks {
        assert_eq!(
            block.get("type").and_then(|t| t.as_str()),
            Some("text"),
            "Anthropic rejects a content block without its discriminant"
        );
    }
}

#[test]
fn full_claude_body_carries_the_marker() {
    let request = LlmRequest {
        model: "claude-sonnet-4-6".into(),
        system: vec![cached_block("stable prefix"), text_block("tail")],
        max_tokens: 1024,
        ..LlmRequest::default()
    };

    let body = VertexProvider::build_claude_body(&request);

    let system = body
        .get("system")
        .and_then(|s| s.as_array())
        .expect("system reaches the body as an array");
    assert!(
        system[0].get("cache_control").is_some(),
        "the marker must survive body construction, not just the helper"
    );
    assert_eq!(
        body.get("anthropic_version").and_then(|v| v.as_str()),
        Some("vertex-2023-10-16"),
        "the rest of the body is unchanged"
    );
}
