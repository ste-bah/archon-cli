//! Tool use over Bedrock ConverseStream.
//!
//! Split out of `bedrock_provider_tests.rs` to keep both files under the
//! 500-line FileSizeGuard threshold.
//!
//! The event carries the block under `start`, NOT `contentBlock` - the latter
//! is the non-streaming Converse response's name. Reading the wrong one matched
//! nothing, so every tool-use block was announced as plain text with no id and
//! no name and no call could be assembled: the model answered prose fine and
//! did nothing whenever a tool was required. There was no test over this event
//! at all, which is how it shipped.

/// The exact payload shape ConverseStream emits, re-wrapped under its
/// `:event-type` header by `decode_eventstream_frames`.
fn tool_use_block_start() -> serde_json::Value {
    serde_json::json!({
        "contentBlockStart": {
            "contentBlockIndex": 1,
            "start": {
                "toolUse": {
                    "toolUseId": "tooluse_KsdM0kAdRhq0oJ",
                    "name": "Read"
                }
            }
        }
    })
}

#[test]
fn bedrock_tool_use_block_start_carries_its_id_and_name() {
    let stream_events =
        archon_llm::providers::bedrock::parse_bedrock_event(&tool_use_block_start());

    assert!(
        matches!(
            stream_events.as_slice(),
            [archon_llm::streaming::StreamEvent::ContentBlockStart {
                index: 1,
                block_type: archon_llm::types::ContentBlockType::ToolUse,
                tool_use_id: Some(id),
                tool_name: Some(name),
            }] if id == "tooluse_KsdM0kAdRhq0oJ" && name == "Read"
        ),
        "tool-use block start must carry id and name, got: {stream_events:?}"
    );
}

/// A start with no `toolUse` is still a text block - the fix must not turn
/// every block into a tool call.
#[test]
fn bedrock_text_block_start_is_not_a_tool_use() {
    let event = serde_json::json!({
        "contentBlockStart": {
            "contentBlockIndex": 0,
            "start": {}
        }
    });

    let stream_events = archon_llm::providers::bedrock::parse_bedrock_event(&event);

    assert!(
        matches!(
            stream_events.as_slice(),
            [archon_llm::streaming::StreamEvent::ContentBlockStart {
                block_type: archon_llm::types::ContentBlockType::Text,
                tool_use_id: None,
                tool_name: None,
                ..
            }]
        ),
        "expected a text block start, got: {stream_events:?}"
    );
}

/// The whole round trip, in the order Bedrock sends it. Asserting the start
/// alone would not have caught this: the argument deltas were always parsed
/// correctly, and it was the missing open call that lost them.
#[test]
fn bedrock_tool_use_sequence_yields_a_named_call_with_arguments() {
    let frames = [
        tool_use_block_start(),
        serde_json::json!({
            "contentBlockDelta": {
                "contentBlockIndex": 1,
                "delta": {"toolUse": {"input": "{\"file_path\":"}}
            }
        }),
        serde_json::json!({
            "contentBlockDelta": {
                "contentBlockIndex": 1,
                "delta": {"toolUse": {"input": "\"/tmp/a.txt\"}"}}
            }
        }),
        serde_json::json!({"contentBlockStop": {"contentBlockIndex": 1}}),
        serde_json::json!({"messageStop": {"stopReason": "tool_use"}}),
    ];

    let stream_events: Vec<_> = frames
        .iter()
        .flat_map(archon_llm::providers::bedrock::parse_bedrock_event)
        .collect();

    let opened_call = stream_events.iter().any(|event| {
        matches!(
            event,
            archon_llm::streaming::StreamEvent::ContentBlockStart {
                block_type: archon_llm::types::ContentBlockType::ToolUse,
                tool_name: Some(name),
                ..
            } if name == "Read"
        )
    });
    assert!(opened_call, "no tool call was opened: {stream_events:?}");

    let arguments: String = stream_events
        .iter()
        .filter_map(|event| match event {
            archon_llm::streaming::StreamEvent::InputJsonDelta { partial_json, .. } => {
                Some(partial_json.as_str())
            }
            _ => None,
        })
        .collect();
    assert_eq!(arguments, "{\"file_path\":\"/tmp/a.txt\"}");

    let stopped_for_tool_use = stream_events.iter().any(|event| {
        matches!(
            event,
            archon_llm::streaming::StreamEvent::MessageDelta {
                stop_reason: Some(reason),
                ..
            } if reason == "tool_use"
        )
    });
    assert!(
        stopped_for_tool_use,
        "expected stop_reason tool_use: {stream_events:?}"
    );
}
