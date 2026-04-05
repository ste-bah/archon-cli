use archon_core::agent::AgentEvent;
use archon_core::input_format::InputFormat;
use archon_core::output_format::{
    OutputFormat, format_agent_event, format_json_result, format_stream_event,
};
use archon_core::print_mode::{
    EXIT_BUDGET_EXCEEDED, EXIT_ERROR, EXIT_MAX_TURNS, EXIT_SUCCESS, PrintModeConfig,
};
use archon_tools::tool::ToolResult;

// ---------------------------------------------------------------------------
// OutputFormat parsing
// ---------------------------------------------------------------------------

#[test]
fn output_format_from_str_text() {
    assert_eq!(OutputFormat::from_str("text"), Ok(OutputFormat::Text));
}

#[test]
fn output_format_from_str_json() {
    assert_eq!(OutputFormat::from_str("json"), Ok(OutputFormat::Json));
}

#[test]
fn output_format_from_str_stream_json() {
    assert_eq!(
        OutputFormat::from_str("stream-json"),
        Ok(OutputFormat::StreamJson)
    );
}

#[test]
fn output_format_from_str_invalid() {
    let result = OutputFormat::from_str("xml");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("xml"));
}

// ---------------------------------------------------------------------------
// InputFormat parsing
// ---------------------------------------------------------------------------

#[test]
fn input_format_from_str_text() {
    assert_eq!(InputFormat::from_str("text"), Ok(InputFormat::Text));
}

#[test]
fn input_format_from_str_stream_json() {
    assert_eq!(
        InputFormat::from_str("stream-json"),
        Ok(InputFormat::StreamJson)
    );
}

#[test]
fn input_format_from_str_invalid() {
    let result = InputFormat::from_str("binary");
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// format_json_result
// ---------------------------------------------------------------------------

#[test]
fn format_json_result_valid_json() {
    let usage = archon_llm::types::Usage {
        input_tokens: 100,
        output_tokens: 200,
        cache_creation_input_tokens: 0,
        cache_read_input_tokens: 0,
    };
    let result = format_json_result("Hello, world!", &usage, 0.05);
    let parsed: serde_json::Value = serde_json::from_str(&result).expect("valid JSON");

    assert_eq!(parsed["content"], "Hello, world!");
    assert_eq!(parsed["usage"]["input_tokens"], 100);
    assert_eq!(parsed["usage"]["output_tokens"], 200);
    assert_eq!(parsed["cost"], 0.05);
}

#[test]
fn format_json_result_empty_content() {
    let usage = archon_llm::types::Usage::default();
    let result = format_json_result("", &usage, 0.0);
    let parsed: serde_json::Value = serde_json::from_str(&result).expect("valid JSON");
    assert_eq!(parsed["content"], "");
}

// ---------------------------------------------------------------------------
// format_stream_event
// ---------------------------------------------------------------------------

#[test]
fn format_stream_event_text() {
    let payload = serde_json::json!({"text": "hello"});
    let line = format_stream_event("text", &payload);
    assert!(line.ends_with('\n'), "NDJSON line must end with newline");

    let parsed: serde_json::Value = serde_json::from_str(line.trim_end()).expect("valid JSON");
    assert_eq!(parsed["type"], "text");
    assert_eq!(parsed["content"]["text"], "hello");
}

#[test]
fn format_stream_event_tool_use() {
    let payload = serde_json::json!({"name": "Bash", "id": "abc123"});
    let line = format_stream_event("tool_use", &payload);
    let parsed: serde_json::Value = serde_json::from_str(line.trim_end()).expect("valid JSON");
    assert_eq!(parsed["type"], "tool_use");
    assert_eq!(parsed["content"]["name"], "Bash");
}

#[test]
fn format_stream_event_error() {
    let payload = serde_json::json!({"message": "something broke"});
    let line = format_stream_event("error", &payload);
    let parsed: serde_json::Value = serde_json::from_str(line.trim_end()).expect("valid JSON");
    assert_eq!(parsed["type"], "error");
    assert_eq!(parsed["content"]["message"], "something broke");
}

// ---------------------------------------------------------------------------
// Exit codes
// ---------------------------------------------------------------------------

#[test]
fn exit_codes_defined() {
    assert_eq!(EXIT_SUCCESS, 0);
    assert_eq!(EXIT_ERROR, 1);
    assert_eq!(EXIT_BUDGET_EXCEEDED, 2);
    assert_eq!(EXIT_MAX_TURNS, 3);
}

// ---------------------------------------------------------------------------
// PrintModeConfig defaults
// ---------------------------------------------------------------------------

#[test]
fn print_mode_config_defaults() {
    let config = PrintModeConfig {
        query: "test query".into(),
        output_format: OutputFormat::Text,
        input_format: InputFormat::Text,
        max_turns: None,
        max_budget_usd: None,
        no_session_persistence: false,
        json_schema: None,
    };

    assert_eq!(config.query, "test query");
    assert_eq!(config.output_format, OutputFormat::Text);
    assert_eq!(config.input_format, InputFormat::Text);
    assert!(config.max_turns.is_none());
    assert!(config.max_budget_usd.is_none());
    assert!(!config.no_session_persistence);
}

#[test]
fn print_mode_config_with_limits() {
    let config = PrintModeConfig {
        query: "hello".into(),
        output_format: OutputFormat::Json,
        input_format: InputFormat::StreamJson,
        max_turns: Some(5),
        max_budget_usd: Some(1.50),
        no_session_persistence: true,
        json_schema: None,
    };

    assert_eq!(config.max_turns, Some(5));
    assert_eq!(config.max_budget_usd, Some(1.50));
    assert!(config.no_session_persistence);
}

// ---------------------------------------------------------------------------
// format_agent_event
// ---------------------------------------------------------------------------

#[test]
fn format_agent_event_text_delta_text_mode() {
    let event = AgentEvent::TextDelta("Hello".into());
    let result = format_agent_event(&event, &OutputFormat::Text);
    assert_eq!(result, Some("Hello".into()));
}

#[test]
fn format_agent_event_text_delta_json_mode() {
    let event = AgentEvent::TextDelta("Hello".into());
    let result = format_agent_event(&event, &OutputFormat::Json);
    // In Json mode, text deltas are accumulated, not emitted per-delta
    assert!(result.is_none());
}

#[test]
fn format_agent_event_text_delta_stream_json_mode() {
    let event = AgentEvent::TextDelta("Hello".into());
    let result = format_agent_event(&event, &OutputFormat::StreamJson);
    assert!(result.is_some());
    let line = result.unwrap();
    let parsed: serde_json::Value = serde_json::from_str(line.trim_end()).expect("valid JSON");
    assert_eq!(parsed["type"], "text");
}

#[test]
fn format_agent_event_error() {
    let event = AgentEvent::Error("something failed".into());
    let result = format_agent_event(&event, &OutputFormat::StreamJson);
    assert!(result.is_some());
    let line = result.unwrap();
    let parsed: serde_json::Value = serde_json::from_str(line.trim_end()).expect("valid JSON");
    assert_eq!(parsed["type"], "error");
}

#[test]
fn format_agent_event_tool_call_started_stream_json() {
    let event = AgentEvent::ToolCallStarted {
        name: "Bash".into(),
        id: "tool_1".into(),
    };
    let result = format_agent_event(&event, &OutputFormat::StreamJson);
    assert!(result.is_some());
    let line = result.unwrap();
    let parsed: serde_json::Value = serde_json::from_str(line.trim_end()).expect("valid JSON");
    assert_eq!(parsed["type"], "tool_use_start");
}

#[test]
fn format_agent_event_tool_call_complete_stream_json() {
    let event = AgentEvent::ToolCallComplete {
        name: "Bash".into(),
        id: "tool_1".into(),
        result: ToolResult::success("output here"),
    };
    let result = format_agent_event(&event, &OutputFormat::StreamJson);
    assert!(result.is_some());
    let line = result.unwrap();
    let parsed: serde_json::Value = serde_json::from_str(line.trim_end()).expect("valid JSON");
    assert_eq!(parsed["type"], "tool_result");
}

#[test]
fn format_agent_event_session_complete_text_mode() {
    let event = AgentEvent::SessionComplete;
    let result = format_agent_event(&event, &OutputFormat::Text);
    assert!(result.is_none());
}

#[test]
fn format_agent_event_turn_complete_stream_json() {
    let event = AgentEvent::TurnComplete {
        input_tokens: 500,
        output_tokens: 200,
    };
    let result = format_agent_event(&event, &OutputFormat::StreamJson);
    assert!(result.is_some());
    let line = result.unwrap();
    let parsed: serde_json::Value = serde_json::from_str(line.trim_end()).expect("valid JSON");
    assert_eq!(parsed["type"], "turn_complete");
}

// ---------------------------------------------------------------------------
// read_input (text mode with simulated input)
// ---------------------------------------------------------------------------

#[test]
fn read_text_input_from_reader() {
    use archon_core::input_format::read_input_from_reader;

    let input = b"Hello, world!\nSecond line" as &[u8];
    let messages = read_input_from_reader(&InputFormat::Text, input).expect("should parse");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0], "Hello, world!\nSecond line");
}

#[test]
fn read_stream_json_input_from_reader() {
    use archon_core::input_format::read_input_from_reader;

    let input =
        b"{\"role\":\"user\",\"content\":\"Hello\"}\n{\"role\":\"user\",\"content\":\"World\"}\n"
            as &[u8];
    let messages = read_input_from_reader(&InputFormat::StreamJson, input).expect("should parse");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0], "Hello");
    assert_eq!(messages[1], "World");
}

#[test]
fn read_stream_json_input_skips_non_user() {
    use archon_core::input_format::read_input_from_reader;

    let input =
        b"{\"role\":\"system\",\"content\":\"ignore\"}\n{\"role\":\"user\",\"content\":\"keep\"}\n"
            as &[u8];
    let messages = read_input_from_reader(&InputFormat::StreamJson, input).expect("should parse");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0], "keep");
}

#[test]
fn read_stream_json_input_invalid_line() {
    use archon_core::input_format::read_input_from_reader;

    let input = b"not valid json\n" as &[u8];
    let result = read_input_from_reader(&InputFormat::StreamJson, input);
    assert!(result.is_err());
}
