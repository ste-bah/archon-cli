use super::{extract_object, reply_entry, reply_entry_with_blocks, resolve_command_block};
use serde_json::json;

#[test]
fn fenced_check_reply_matches_shared_fixture_and_inline_history() {
    let fixture = include_str!("fixtures/decompose_command_block_reply.txt");
    let text = extract_object(fixture);
    assert_eq!(
        text,
        r#"{"id":"SUP-REQ-X","check":{"kind":"command","command_block":"SUP-REQ-X"}}"#
    );
    let entry = reply_entry_with_blocks(fixture, "SUP-REQ-X").unwrap();
    assert_eq!(
        entry["check"]["command"],
        "printf '%s' \"quote \\\" slash \\\\ literal \\\\n\"\necho ```"
    );
    assert!(entry["check"].get("command_block").is_none());
    assert_eq!(
        resolve_command_block(
            &mut json!({"id":"A","check":{"kind":"command","command":"run"}}),
            "```check X\nrun\n```"
        ),
        Some("acceptance reply contains unreferenced check block X".into())
    );

    let inline = r#"```json
{"id":"A","check":{"kind":"command","command":"printf \\\"ok\\\"\\n"}}
```"#;
    let inline_text = extract_object(inline);
    let inline_entry = reply_entry_with_blocks(inline, "A").unwrap();
    assert_eq!(inline_entry, reply_entry(&inline_text, "A").unwrap());
}

#[test]
fn non_entry_json_replies_are_unreadable_without_panicking() {
    let replies: Vec<String> =
        serde_json::from_str(include_str!("fixtures/decompose_non_entry_replies.json")).unwrap();
    for reply in replies {
        assert_eq!(
            reply_entry_with_blocks(&reply, "A"),
            None,
            "non-entry reply should follow the missing-entry path: {reply}"
        );
    }
}
