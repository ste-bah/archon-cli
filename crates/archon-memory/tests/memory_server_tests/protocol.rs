use super::*;
// Protocol tests
// ═══════════════════════════════════════════════════════════════

#[test]
fn serialize_request() {
    let json = make_request(1, "ping", serde_json::json!({}));
    assert!(json.ends_with('\n'), "must be newline-terminated");
    let parsed: Request = serde_json::from_str(json.trim()).expect("valid JSON");
    assert_eq!(parsed.id, 1);
    assert_eq!(parsed.method, "ping");
}

#[test]
fn parse_response_ok() {
    let line = r#"{"id":1,"result":"pong","error":null}"#;
    let resp = parse_response(line).expect("parse ok");
    assert_eq!(resp.id, 1);
    assert_eq!(resp.result, Some(serde_json::json!("pong")));
    assert!(resp.error.is_none());
}

#[test]
fn parse_response_error() {
    let line = r#"{"id":2,"result":null,"error":{"message":"not found"}}"#;
    let resp = parse_response(line).expect("parse error response");
    assert_eq!(resp.id, 2);
    assert!(resp.result.is_none());
    assert_eq!(resp.error.as_ref().expect("has error").message, "not found");
}

#[test]
fn parse_response_malformed() {
    let result = parse_response("not json at all {{{");
    assert!(result.is_err());
}

// ═══════════════════════════════════════════════════════════════
