use super::describe;

fn parse_error(text: &str) -> serde_json::Error {
    serde_json::from_str::<serde_json::Value>(text).expect_err("invalid JSON")
}

#[test]
fn invalid_escape_diagnostic_marks_the_fault_and_teaches_json_backslashes() {
    let reply = r#"{"id":"A","label":"bad\    if"}"#;
    let error = parse_error(reply);
    let diagnostic = describe(reply.as_bytes(), &error);
    assert!(diagnostic.contains("Syntax"), "{diagnostic}");
    assert!(diagnostic.contains(&format!("line {} column {}", error.line(), error.column())));
    assert!(diagnostic.contains("<HERE>"), "{diagnostic}");
    assert!(
        diagnostic.contains("literal backslash inside a JSON string must be written as `\\\\`"),
        "{diagnostic}"
    );
}

#[test]
fn invalid_escape_marker_stays_at_the_escape() {
    let reply = r#"{"a":"\q"}"#;
    let error = parse_error(reply);
    let diagnostic = describe(reply.as_bytes(), &error);
    assert!(
        diagnostic.contains(r#"{"a":"<HERE>\q"}"#) || diagnostic.contains(r#"{"a":"\<HERE>q"}"#),
        "{diagnostic}"
    );
}

#[test]
fn eof_diagnostic_places_marker_at_end_of_input() {
    let reply = r#"{"x":"#;
    let error = parse_error(reply);
    assert_eq!(error.classify(), serde_json::error::Category::Eof);
    let diagnostic = describe(reply.as_bytes(), &error);
    assert!(diagnostic.contains("near `{\"x\":<HERE>`"), "{diagnostic}");
}

#[test]
fn multibyte_text_near_the_error_is_sliced_without_panicking() {
    let reply = r#"{"label":"é🦀é","bad":"\q"}"#;
    let error = parse_error(reply);
    let diagnostic = describe(reply.as_bytes(), &error);
    assert!(diagnostic.contains("é🦀é"), "{diagnostic}");
    assert!(diagnostic.contains("<HERE>"), "{diagnostic}");
}

#[test]
fn very_long_reply_diagnostic_stays_bounded() {
    let reply = format!(r#"{{"padding":"{}","bad":"\q"}}"#, "a".repeat(50_000));
    let error = parse_error(&reply);
    let diagnostic = describe(reply.as_bytes(), &error);
    assert!(diagnostic.len() < 700, "{} bytes", diagnostic.len());
    assert!(diagnostic.contains("<HERE>"), "{diagnostic}");
}

#[test]
fn refusal_redacts_secret_shaped_text_omits_marker_and_preserves_location() {
    let reply = r#"{"check":{"command":"x-api-key: abc123 \q"}}"#;
    let error = parse_error(reply);
    let diagnostic = describe(reply.as_bytes(), &error);
    assert!(!diagnostic.contains("abc123"), "{diagnostic}");
    assert!(diagnostic.contains("<redacted>"), "{diagnostic}");
    assert!(!diagnostic.contains("<HERE>"), "{diagnostic}");
    assert!(
        diagnostic.contains(&format!("line {} column {}", error.line(), error.column())),
        "{diagnostic}"
    );
}

#[test]
fn redaction_runs_across_the_fault_before_adding_a_marker() {
    let reply = r#"{"api-\qkey":"secret123"}"#;
    let error = parse_error(reply);
    let diagnostic = describe(reply.as_bytes(), &error);
    assert!(!diagnostic.contains("secret123"), "{diagnostic}");
    assert!(!diagnostic.contains("<HERE>"), "{diagnostic}");
    assert!(
        diagnostic.contains(r#"{"api-key":"<redacted>"}"#),
        "{diagnostic}"
    );
    assert!(diagnostic.contains(&format!("line {} column {}", error.line(), error.column())));
}

#[test]
fn invalid_escape_excerpt_matches_the_javascript_refusal_window() {
    let reply = r#"{"id":"A","check":{"kind":"command","command":"echo \q"}}"#;
    let error = parse_error(reply);
    let diagnostic = describe(reply.as_bytes(), &error);
    assert!(diagnostic.contains("<HERE>"), "{diagnostic}");
    assert!(
        diagnostic.contains(r#"near `eck":{"kind":"command","command":"echo \<HERE>q"}}`"#),
        "{diagnostic}"
    );
    assert!(
        diagnostic.contains("a literal backslash inside a JSON string must be written as `\\\\`"),
        "{diagnostic}"
    );
}

#[test]
fn invalid_escape_inside_multiline_command_recommends_block_form() {
    let reply = r#"{"id":"A","check":{"kind":"command","command":"set -eu\nprintf x\necho \q"}}"#;
    let error = parse_error(reply);
    let diagnostic = describe(reply.as_bytes(), &error);
    assert!(diagnostic.contains(&format!("line {} column {}", error.line(), error.column())));
    assert!(
        diagnostic.contains("literal backslash inside a JSON string must be written as `\\\\`")
    );
    assert!(
        diagnostic.contains("the check is a multi-line script; return it in block form instead:")
    );
    assert!(diagnostic.contains("set \"command_block\": true in the entry"));
    assert!(diagnostic.contains("fence labelled check after the JSON entry"));
}

#[test]
fn syntax_error_outside_command_keeps_the_existing_diagnostic() {
    let reply = r#"{"id":"A","label":"bad\q","check":{"kind":"command","command":"echo ok"}}"#;
    let error = parse_error(reply);
    let diagnostic = describe(reply.as_bytes(), &error);
    assert!(!diagnostic.contains("multi-line script"), "{diagnostic}");
    assert!(
        diagnostic.contains("literal backslash inside a JSON string must be written as `\\\\`")
    );
}

#[test]
fn escaped_newline_in_command_recommends_block_form_even_when_error_is_elsewhere() {
    let reply =
        r#"{"id":"A","check":{"kind":"command","command":"first\nsecond"},"label":"bad\q"}"#;
    let error = parse_error(reply);
    let diagnostic = describe(reply.as_bytes(), &error);
    assert!(diagnostic.contains("multi-line script"), "{diagnostic}");
}

#[test]
fn shared_command_block_form_direction_cases() {
    const DIRECTION: &str = "the check is a multi-line script; return it in block form instead: set \"command_block\": true in the entry and put the script unescaped in exactly one fence labelled check after the JSON entry; then no escaping is needed";
    for (line_number, line) in include_str!("workflow_parse_block_form_direction_cases.jsonl")
        .lines()
        .enumerate()
    {
        let case: serde_json::Value = serde_json::from_str(line)
            .unwrap_or_else(|error| panic!("fixture line {}: {error}", line_number + 1));
        let reply = case["reply"].as_str().expect("reply string");
        let expected = case["expectedBlockFormDirection"]
            .as_str()
            .expect("expectedBlockFormDirection string");
        let error = parse_error(reply);
        let diagnostic = describe(reply.as_bytes(), &error);
        // Only the block-form direction is shared: parser-generated refusal text differs legitimately between V8 and serde.
        assert_eq!(
            if diagnostic.contains(DIRECTION) {
                DIRECTION
            } else {
                ""
            },
            expected,
            "fixture line {} produced: {diagnostic}",
            line_number + 1
        );
    }
}
