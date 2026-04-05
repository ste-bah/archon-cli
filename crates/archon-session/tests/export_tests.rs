use serde_json::json;

use archon_session::export::{ExportFormat, export_session};

// ---------------------------------------------------------------------------
// ExportFormat::from_str
// ---------------------------------------------------------------------------

#[test]
fn export_format_from_str() {
    assert!(matches!(
        ExportFormat::from_str("markdown"),
        Ok(ExportFormat::Markdown)
    ));
    assert!(matches!(
        ExportFormat::from_str("json"),
        Ok(ExportFormat::Json)
    ));
    assert!(matches!(
        ExportFormat::from_str("text"),
        Ok(ExportFormat::Text)
    ));
    // case-insensitive
    assert!(matches!(
        ExportFormat::from_str("Markdown"),
        Ok(ExportFormat::Markdown)
    ));
    assert!(matches!(
        ExportFormat::from_str("JSON"),
        Ok(ExportFormat::Json)
    ));
}

#[test]
fn export_format_invalid() {
    let result = ExportFormat::from_str("xml");
    assert!(result.is_err());
    let msg = result.unwrap_err();
    assert!(
        msg.contains("xml"),
        "error should mention the invalid format"
    );
}

// ---------------------------------------------------------------------------
// Markdown export
// ---------------------------------------------------------------------------

fn sample_messages() -> Vec<serde_json::Value> {
    vec![
        json!({ "role": "user", "content": "Hello, world!" }),
        json!({ "role": "assistant", "content": "Hi there!" }),
        json!({ "role": "user", "content": "How are you?" }),
        json!({ "role": "assistant", "content": "I am fine, thank you." }),
    ]
}

#[test]
fn export_markdown_has_headers() {
    let output = export_session(&sample_messages(), "sess-abc", ExportFormat::Markdown)
        .expect("export should succeed");
    assert!(
        output.contains("# Session"),
        "markdown should contain '# Session' header"
    );
}

#[test]
fn export_markdown_has_turns() {
    let output = export_session(&sample_messages(), "sess-abc", ExportFormat::Markdown)
        .expect("export should succeed");
    assert!(
        output.contains("## Turn"),
        "markdown should contain '## Turn' headers"
    );
}

// ---------------------------------------------------------------------------
// JSON export
// ---------------------------------------------------------------------------

#[test]
fn export_json_valid() {
    let output = export_session(&sample_messages(), "sess-abc", ExportFormat::Json)
        .expect("export should succeed");
    let parsed: serde_json::Value =
        serde_json::from_str(&output).expect("output should be valid JSON");
    assert!(parsed.is_object(), "top level should be an object");
}

#[test]
fn export_json_has_messages() {
    let output = export_session(&sample_messages(), "sess-abc", ExportFormat::Json)
        .expect("export should succeed");
    let parsed: serde_json::Value =
        serde_json::from_str(&output).expect("output should be valid JSON");
    let messages = parsed
        .get("messages")
        .and_then(|m| m.as_array())
        .expect("should have a 'messages' array");
    assert_eq!(messages.len(), 4);
}

// ---------------------------------------------------------------------------
// Text export
// ---------------------------------------------------------------------------

#[test]
fn export_text_has_roles() {
    let output = export_session(&sample_messages(), "sess-abc", ExportFormat::Text)
        .expect("export should succeed");
    assert!(output.contains("User:"), "text should contain 'User:'");
    assert!(
        output.contains("Assistant:"),
        "text should contain 'Assistant:'"
    );
}

// ---------------------------------------------------------------------------
// Edge case: empty messages
// ---------------------------------------------------------------------------

#[test]
fn export_empty_messages() {
    let empty: Vec<serde_json::Value> = vec![];
    // All three formats should succeed on empty input without panicking
    let md = export_session(&empty, "empty-sess", ExportFormat::Markdown);
    assert!(md.is_ok());
    let js = export_session(&empty, "empty-sess", ExportFormat::Json);
    assert!(js.is_ok());
    let txt = export_session(&empty, "empty-sess", ExportFormat::Text);
    assert!(txt.is_ok());
}
