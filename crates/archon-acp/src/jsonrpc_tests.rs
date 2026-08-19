//! Framing tests (#189 Phase 11).

use super::*;

#[test]
fn a_call_with_an_id_is_a_request() {
    let message: Incoming =
        serde_json::from_str(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#)
            .expect("parses");

    assert!(message.is_request());
    assert!(!message.is_notification());
    assert!(!message.is_reply());
}

/// The distinction that matters: `session/cancel` has no id, and treating it
/// as a request would mean replying to something that expects no reply.
#[test]
fn a_call_without_an_id_is_a_notification() {
    let message: Incoming = serde_json::from_str(
        r#"{"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":"s1"}}"#,
    )
    .expect("parses");

    assert!(message.is_notification());
    assert!(!message.is_request());
}

#[test]
fn a_message_with_an_id_and_no_method_is_a_reply() {
    let message: Incoming =
        serde_json::from_str(r#"{"jsonrpc":"2.0","id":7,"result":{"outcome":{}}}"#)
            .expect("parses");

    assert!(message.is_reply());
    assert!(message.result.is_some());
}

#[test]
fn a_response_carries_the_version_and_the_id_it_answers() {
    let response = Response::ok(serde_json::json!(3), serde_json::json!({"ok": true}));
    let value = serde_json::to_value(&response).expect("serialises");

    assert_eq!(value["jsonrpc"], "2.0");
    assert_eq!(value["id"], 3);
    assert_eq!(value["result"]["ok"], true);
    assert!(
        value.get("error").is_none(),
        "a success must not also carry an error: {value}"
    );
}

#[test]
fn an_error_response_carries_no_result() {
    let response = Response::err(serde_json::json!(4), METHOD_NOT_FOUND, "no such method");
    let value = serde_json::to_value(&response).expect("serialises");

    assert_eq!(value["error"]["code"], METHOD_NOT_FOUND);
    assert_eq!(value["error"]["message"], "no such method");
    assert!(value.get("result").is_none(), "{value}");
}

/// One value per line is the whole framing contract. A message containing a
/// raw newline would be read as two, and both halves would be unparseable.
#[test]
fn a_serialised_message_never_contains_a_newline() {
    let notification = Notification::new(
        "session/update",
        serde_json::json!({ "text": "first\nsecond\nthird" }),
    );

    let line = serde_json::to_string(&notification).expect("serialises");

    assert!(!line.contains('\n'), "{line}");
    assert!(
        line.contains("\\n"),
        "the newline must survive escaped: {line}"
    );
}

#[test]
fn a_request_is_numbered_so_its_reply_can_be_matched() {
    let request = Request::new(42, "session/request_permission", serde_json::json!({}));
    let value = serde_json::to_value(&request).expect("serialises");

    assert_eq!(value["id"], 42);
    assert_eq!(value["method"], "session/request_permission");
}
