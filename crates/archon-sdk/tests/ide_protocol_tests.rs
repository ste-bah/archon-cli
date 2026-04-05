//! IDE Protocol tests for TASK-CLI-411.
//! Tests JSON-RPC 2.0 framing, all IDE message types, and the stdio transport.

use archon_sdk::ide::protocol::{
    IdeCapabilities, IdeInitializeParams, IdeClientInfo, IdeInitializeResult,
    IdePromptParams, IdeTextDelta, IdeTurnComplete,
    JRpcRequest, JRpcNotification, JRpcErrorCode,
    parse_request, error_response,
};
use archon_sdk::ide::handler::IdeProtocolHandler;
use archon_sdk::ide::stdio::StdioTransport;
use std::io::Cursor;

// ── 1. JSON-RPC request serialization ────────────────────────────────────────

#[test]
fn json_rpc_request_serialization() {
    let params = serde_json::json!({"clientInfo": {"name": "test", "version": "1.0"}});
    let req = JRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: 1,
        method: "archon/initialize".to_string(),
        params,
    };
    let json = serde_json::to_string(&req).expect("serialize");
    let v: serde_json::Value = serde_json::from_str(&json).expect("parse");
    assert_eq!(v["jsonrpc"], "2.0");
    assert_eq!(v["id"], 1);
    assert_eq!(v["method"], "archon/initialize");
}

// ── 2. JSON-RPC response deserialization ─────────────────────────────────────

#[test]
fn json_rpc_response_deserialization() {
    let json = r#"{"jsonrpc":"2.0","id":1,"result":{"sessionId":"abc"}}"#;
    let v: serde_json::Value = serde_json::from_str(json).expect("parse");
    assert_eq!(v["jsonrpc"], "2.0");
    assert_eq!(v["id"], 1);
    assert_eq!(v["result"]["sessionId"], "abc");
}

// ── 3. JSON-RPC error deserialization ────────────────────────────────────────

#[test]
fn json_rpc_error_deserialization() {
    let json = r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32600,"message":"Invalid Request"}}"#;
    let v: serde_json::Value = serde_json::from_str(json).expect("parse");
    assert_eq!(v["jsonrpc"], "2.0");
    assert_eq!(v["id"], 1);
    assert_eq!(v["error"]["code"], -32600);
    assert_eq!(v["error"]["message"], "Invalid Request");
}

// ── 4. JSON-RPC notification has no "id" field ───────────────────────────────

#[test]
fn json_rpc_notification_no_id() {
    let notif = JRpcNotification {
        jsonrpc: "2.0".to_string(),
        method: "archon/textDelta".to_string(),
        params: serde_json::json!({"sessionId": "s1", "text": "hello"}),
    };
    let json = serde_json::to_string(&notif).expect("serialize");
    let v: serde_json::Value = serde_json::from_str(&json).expect("parse");
    assert_eq!(v["jsonrpc"], "2.0");
    assert_eq!(v["method"], "archon/textDelta");
    assert!(v.get("id").is_none(), "notifications must not have an 'id' field");
}

// ── 5. IdeInitializeParams roundtrip serde ───────────────────────────────────

#[test]
fn initialize_request_params() {
    let params = IdeInitializeParams {
        client_info: IdeClientInfo {
            name: "vscode-archon".to_string(),
            version: "0.5.0".to_string(),
        },
        capabilities: IdeCapabilities {
            inline_completion: true,
            tool_execution: true,
            diff: false,
            terminal: false,
        },
    };
    let json = serde_json::to_string(&params).expect("serialize");
    let decoded: IdeInitializeParams = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(decoded.client_info.name, "vscode-archon");
    assert_eq!(decoded.client_info.version, "0.5.0");
    assert!(decoded.capabilities.inline_completion);
    assert!(decoded.capabilities.tool_execution);
    assert!(!decoded.capabilities.diff);
}

// ── 6. IdeInitializeResult has sessionId field ───────────────────────────────

#[test]
fn initialize_result_has_session_id() {
    let result = IdeInitializeResult {
        session_id: "sess-abc-123".to_string(),
        server_version: "0.1.0".to_string(),
        capabilities: IdeCapabilities::default(),
    };
    let json = serde_json::to_string(&result).expect("serialize");
    let v: serde_json::Value = serde_json::from_str(&json).expect("parse");
    assert_eq!(v["sessionId"], "sess-abc-123");
    assert_eq!(v["serverVersion"], "0.1.0");
}

// ── 7. IdePromptParams roundtrip serde ───────────────────────────────────────

#[test]
fn prompt_request_params() {
    let params = IdePromptParams {
        session_id: "sess-xyz".to_string(),
        text: "What is 2 + 2?".to_string(),
        context_files: None,
    };
    let json = serde_json::to_string(&params).expect("serialize");
    let decoded: IdePromptParams = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(decoded.session_id, "sess-xyz");
    assert_eq!(decoded.text, "What is 2 + 2?");
    assert!(decoded.context_files.is_none());
}

// ── 8. IdeTextDelta roundtrip serde ──────────────────────────────────────────

#[test]
fn text_delta_notification() {
    let delta = IdeTextDelta {
        session_id: "sess-1".to_string(),
        text: "Hello, world!".to_string(),
    };
    let json = serde_json::to_string(&delta).expect("serialize");
    let decoded: IdeTextDelta = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(decoded.session_id, "sess-1");
    assert_eq!(decoded.text, "Hello, world!");
}

// ── 9. IdeTurnComplete roundtrip serde ───────────────────────────────────────

#[test]
fn turn_complete_notification() {
    let complete = IdeTurnComplete {
        session_id: "sess-2".to_string(),
        input_tokens: 100,
        output_tokens: 200,
        cost: 0.003,
    };
    let json = serde_json::to_string(&complete).expect("serialize");
    let decoded: IdeTurnComplete = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(decoded.session_id, "sess-2");
    assert_eq!(decoded.input_tokens, 100);
    assert_eq!(decoded.output_tokens, 200);
    assert!((decoded.cost - 0.003).abs() < 1e-9);
}

// ── 10. IdeCapabilities::default() has all fields false ──────────────────────

#[test]
fn capabilities_default() {
    let caps = IdeCapabilities::default();
    assert!(!caps.inline_completion);
    assert!(!caps.tool_execution);
    assert!(!caps.diff);
    assert!(!caps.terminal);
}

// ── 11. Stdio transport roundtrip ────────────────────────────────────────────

#[test]
fn stdio_transport_roundtrip() {
    let handler = IdeProtocolHandler::new("0.1.0");
    let mut transport = StdioTransport::new(handler);

    // Build a valid JSON-RPC initialize request
    let request_line = r#"{"jsonrpc":"2.0","id":42,"method":"archon/initialize","params":{"clientInfo":{"name":"test","version":"1.0"},"capabilities":{"inlineCompletion":false,"toolExecution":false,"diff":false,"terminal":false}}}"#;
    let input = format!("{}\n", request_line);
    let reader = Cursor::new(input.into_bytes());
    let mut output = Vec::new();

    transport.run(reader, &mut output).expect("run transport");

    let response_str = String::from_utf8(output).expect("utf8");
    let response_line = response_str.trim();
    assert!(!response_line.is_empty(), "expected a response line");

    let v: serde_json::Value = serde_json::from_str(response_line).expect("valid JSON response");
    assert_eq!(v["jsonrpc"], "2.0");
    assert_eq!(v["id"], 42, "response id must match request id");
    assert!(v.get("result").is_some(), "expected result field");
}

// ── 12. JRpcErrorCode::InvalidRequest == -32600 ──────────────────────────────

#[test]
fn error_code_invalid_request() {
    assert_eq!(JRpcErrorCode::INVALID_REQUEST, -32600);
    assert_eq!(JRpcErrorCode::PARSE_ERROR, -32700);
    assert_eq!(JRpcErrorCode::METHOD_NOT_FOUND, -32601);
    assert_eq!(JRpcErrorCode::INVALID_PARAMS, -32602);
    assert_eq!(JRpcErrorCode::INTERNAL_ERROR, -32603);

    // error_response helper produces valid JSON-RPC error
    let resp = error_response(99, JRpcErrorCode::INVALID_REQUEST, "bad request");
    let v: serde_json::Value = serde_json::from_str(&resp).expect("valid JSON");
    assert_eq!(v["id"], 99);
    assert_eq!(v["error"]["code"], -32600);
    assert_eq!(v["error"]["message"], "bad request");

    // parse_request works on well-formed input
    let req_json = r#"{"jsonrpc":"2.0","id":5,"method":"archon/status","params":{"sessionId":"s"}}"#;
    let (id, method, _params) = parse_request(req_json).expect("parse");
    assert_eq!(id, 5);
    assert_eq!(method, "archon/status");
}
