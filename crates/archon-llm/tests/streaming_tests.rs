use archon_llm::streaming::{StreamEvent, parse_sse_event, split_sse_lines};
use archon_llm::types::ContentBlockType;

#[test]
fn parse_message_start() {
    let data = r#"{"type":"message_start","message":{"id":"msg_01abc","type":"message","role":"assistant","content":[],"model":"claude-sonnet-4-6","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":100,"output_tokens":0}}}"#;

    match parse_sse_event("message_start", data).expect("parse") {
        StreamEvent::MessageStart { id, model, usage } => {
            assert_eq!(id, "msg_01abc");
            assert_eq!(model, "claude-sonnet-4-6");
            assert_eq!(usage.input_tokens, 100);
            assert_eq!(usage.output_tokens, 0);
        }
        other => panic!("wrong event: {other:?}"),
    }
}

#[test]
fn parse_content_block_start_text() {
    let data =
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#;

    match parse_sse_event("content_block_start", data).expect("parse") {
        StreamEvent::ContentBlockStart {
            index, block_type, ..
        } => {
            assert_eq!(index, 0);
            assert_eq!(block_type, ContentBlockType::Text);
        }
        other => panic!("wrong event: {other:?}"),
    }
}

#[test]
fn parse_content_block_start_thinking() {
    let data = r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}"#;

    match parse_sse_event("content_block_start", data).expect("parse") {
        StreamEvent::ContentBlockStart { block_type, .. } => {
            assert_eq!(block_type, ContentBlockType::Thinking);
        }
        other => panic!("wrong event: {other:?}"),
    }
}

#[test]
fn parse_content_block_start_tool_use() {
    let data = r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_01xyz","name":"Read","input":{}}}"#;

    match parse_sse_event("content_block_start", data).expect("parse") {
        StreamEvent::ContentBlockStart {
            index,
            block_type,
            tool_use_id,
            tool_name,
        } => {
            assert_eq!(index, 1);
            assert_eq!(block_type, ContentBlockType::ToolUse);
            assert_eq!(tool_use_id.as_deref(), Some("toolu_01xyz"));
            assert_eq!(tool_name.as_deref(), Some("Read"));
        }
        other => panic!("wrong event: {other:?}"),
    }
}

#[test]
fn parse_text_delta() {
    let data = r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello, "}}"#;

    match parse_sse_event("content_block_delta", data).expect("parse") {
        StreamEvent::TextDelta { index, text } => {
            assert_eq!(index, 0);
            assert_eq!(text, "Hello, ");
        }
        other => panic!("wrong event: {other:?}"),
    }
}

#[test]
fn parse_thinking_delta() {
    let data = r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Let me think..."}}"#;

    match parse_sse_event("content_block_delta", data).expect("parse") {
        StreamEvent::ThinkingDelta { thinking, .. } => {
            assert_eq!(thinking, "Let me think...");
        }
        other => panic!("wrong event: {other:?}"),
    }
}

#[test]
fn parse_input_json_delta() {
    let data = r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"file_path\":"}}"#;

    match parse_sse_event("content_block_delta", data).expect("parse") {
        StreamEvent::InputJsonDelta {
            index,
            partial_json,
        } => {
            assert_eq!(index, 1);
            assert!(partial_json.contains("file_path"));
        }
        other => panic!("wrong event: {other:?}"),
    }
}

#[test]
fn parse_signature_delta() {
    let data = r#"{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"abc123"}}"#;

    match parse_sse_event("content_block_delta", data).expect("parse") {
        StreamEvent::SignatureDelta { signature, .. } => {
            assert_eq!(signature, "abc123");
        }
        other => panic!("wrong event: {other:?}"),
    }
}

#[test]
fn parse_content_block_stop() {
    let data = r#"{"type":"content_block_stop","index":0}"#;

    match parse_sse_event("content_block_stop", data).expect("parse") {
        StreamEvent::ContentBlockStop { index } => {
            assert_eq!(index, 0);
        }
        other => panic!("wrong event: {other:?}"),
    }
}

#[test]
fn parse_message_delta() {
    let data = r#"{"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":42}}"#;

    match parse_sse_event("message_delta", data).expect("parse") {
        StreamEvent::MessageDelta { stop_reason, usage } => {
            assert_eq!(stop_reason.as_deref(), Some("end_turn"));
            assert_eq!(usage.as_ref().map(|u| u.output_tokens), Some(42));
        }
        other => panic!("wrong event: {other:?}"),
    }
}

#[test]
fn parse_message_stop() {
    match parse_sse_event("message_stop", "{}").expect("parse") {
        StreamEvent::MessageStop => {}
        other => panic!("wrong event: {other:?}"),
    }
}

#[test]
fn parse_ping() {
    match parse_sse_event("ping", "{}").expect("parse") {
        StreamEvent::Ping => {}
        other => panic!("wrong event: {other:?}"),
    }
}

#[test]
fn parse_error_event() {
    let data = r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#;

    match parse_sse_event("error", data).expect("parse") {
        StreamEvent::Error {
            error_type,
            message,
        } => {
            assert_eq!(error_type, "overloaded_error");
            assert_eq!(message, "Overloaded");
        }
        other => panic!("wrong event: {other:?}"),
    }
}

#[test]
fn unknown_event_type_returns_error() {
    let result = parse_sse_event("unknown_event", "{}");
    assert!(result.is_err());
}

#[test]
fn malformed_data_returns_error_not_panic() {
    let result = parse_sse_event("message_start", "not json at all");
    assert!(result.is_err());

    let result = parse_sse_event("content_block_delta", "{broken");
    assert!(result.is_err());
}

// -----------------------------------------------------------------------
// SSE line splitting
// -----------------------------------------------------------------------

#[test]
fn split_sse_lines_basic() {
    let raw = "\
event: message_start
data: {\"type\":\"message_start\"}

event: content_block_delta
data: {\"type\":\"content_block_delta\"}

event: message_stop
data: {}
";

    let pairs = split_sse_lines(raw);
    assert_eq!(pairs.len(), 3);
    assert_eq!(pairs[0].0, "message_start");
    assert_eq!(pairs[1].0, "content_block_delta");
    assert_eq!(pairs[2].0, "message_stop");
}

#[test]
fn split_sse_handles_ping() {
    let raw = "\
event: ping
data: {}
";
    let pairs = split_sse_lines(raw);
    assert_eq!(pairs.len(), 1);
    assert_eq!(pairs[0].0, "ping");
}

// -----------------------------------------------------------------------
// Tool call JSON accumulation
// -----------------------------------------------------------------------

#[test]
fn accumulate_tool_call_json() {
    // Simulate multiple input_json_delta events
    let deltas = [
        r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"file"}}"#,
        r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"_path\": \""}}"#,
        r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"test.rs\"}"}}"#,
    ];

    let mut accumulated = String::new();
    for data in &deltas {
        match parse_sse_event("content_block_delta", data).expect("parse") {
            StreamEvent::InputJsonDelta { partial_json, .. } => {
                accumulated.push_str(&partial_json);
            }
            other => panic!("wrong event: {other:?}"),
        }
    }

    let parsed: serde_json::Value =
        serde_json::from_str(&accumulated).expect("accumulated JSON should be valid");
    assert_eq!(parsed["file_path"].as_str(), Some("test.rs"));
}

// -----------------------------------------------------------------------
// Usage tracking
// -----------------------------------------------------------------------

#[test]
fn usage_merge() {
    let mut total = archon_llm::types::Usage::default();

    // From message_start
    let start_usage = archon_llm::types::Usage {
        input_tokens: 100,
        output_tokens: 0,
        cache_creation_input_tokens: 50,
        cache_read_input_tokens: 25,
    };
    total.merge(&start_usage);

    // From message_delta
    let delta_usage = archon_llm::types::Usage {
        input_tokens: 0,
        output_tokens: 200,
        cache_creation_input_tokens: 0,
        cache_read_input_tokens: 0,
    };
    total.merge(&delta_usage);

    assert_eq!(total.input_tokens, 100);
    assert_eq!(total.output_tokens, 200);
    assert_eq!(total.cache_creation_input_tokens, 50);
    assert_eq!(total.cache_read_input_tokens, 25);
}
