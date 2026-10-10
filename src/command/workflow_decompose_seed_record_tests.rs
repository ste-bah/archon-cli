use super::{extract_object, reply_entry, reply_entry_with_blocks, resolve_command_block};
use serde_json::json;

#[derive(serde::Deserialize)]
struct ValueFixture {
    id: String,
    reply: String,
    expected: String,
}

#[derive(serde::Deserialize)]
struct CandidateFixture {
    id: String,
    reply: String,
    expected: String,
}

#[test]
fn malformed_extra_json_candidate_is_refused_but_duplicates_and_single_errors_keep_their_rules() {
    let fixtures: Vec<CandidateFixture> = serde_json::from_str(include_str!(
        "fixtures/decompose_json_candidate_outcomes.json"
    ))
    .unwrap();
    for fixture in fixtures {
        match fixture.id.as_str() {
            "mixed-invalid" => {
                assert_eq!(extract_object(&fixture.reply), "", "{}", fixture.expected);
                assert!(reply_entry_with_blocks(&fixture.reply, "A").is_none());
            }
            "identical-valid" => {
                assert!(
                    reply_entry_with_blocks(&fixture.reply, "A").is_some(),
                    "{}",
                    fixture.expected
                );
            }
            "single-invalid" => {
                let object = extract_object(&fixture.reply);
                assert_eq!(reply_entry(&object, "A"), None, "{}", fixture.expected);
                assert!(object.contains("\\q"));
            }
            other => panic!("unexpected JSON candidate fixture {other}"),
        }
    }
}

#[test]
fn check_fence_contract_and_identical_entry_duplicate_are_shared() {
    let content = "```json\n{\"id\":\"A\",\"check\":{\"kind\":\"command\",\"command_block\":true}}\n```\n```check\nrun\necho done\ntrue\n```";
    let text = extract_object(content);
    let entry = reply_entry_with_blocks(content, "A").unwrap();
    assert_eq!(entry["check"]["command"], "run\necho done\ntrue");
    assert!(entry["check"].get("command_block").is_none());
    assert_eq!(reply_entry(&text, "A").unwrap()["id"], "A");

    let duplicate = "{\"id\":\"A\",\"check\":{\"kind\":\"command\",\"command\":\"run\"}}\n```json\n{\"check\":{\"command\":\"run\",\"kind\":\"command\"},\"id\":\"A\"}\n```";
    assert_eq!(reply_entry_with_blocks(duplicate, "A").unwrap()["id"], "A");
    let conflicting = "{\"id\":\"A\"}\n```json\n{\"id\":\"B\"}\n```";
    assert!(reply_entry_with_blocks(conflicting, "A").is_none());
    let multiple = "```json\n[{\"id\":\"A\"},{\"id\":\"B\"}]\n```";
    assert!(reply_entry_with_blocks(multiple, "A").is_none());
}

#[test]
fn check_block_refusals_and_inline_command_match_contract() {
    assert_eq!(
        resolve_command_block(
            &mut json!({"id":"A","check":{"kind":"command","command_block":true}}),
            "{}"
        ),
        Some("acceptance entry A check.command_block is true but no check block is present".into())
    );
    assert_eq!(
        resolve_command_block(
            &mut json!({"id":"A","check":{"kind":"command","command_block":true}}),
            "```check\na\n```\n```check\nb\n```"
        ),
        Some("acceptance reply contains more than one check block".into())
    );
    assert_eq!(
        resolve_command_block(&mut json!({"id":"A","check":{"kind":"command","command":"run"}}), "```check\nrun\n```"),
        Some("check block present but check.command_block is not true — put the script only in the block and set command_block true, or remove the block".into())
    );
    assert_eq!(
        resolve_command_block(
            &mut json!({"id":"A","check":{"kind":"command","command_block":true}}),
            "```check\n```"
        ),
        Some("acceptance entry A check block is empty".into())
    );
    assert_eq!(
        resolve_command_block(
            &mut json!({"id":"A","check":{"kind":"command","command":"run"}}),
            "```check named\nignored\n```"
        ),
        None
    );
    assert_eq!(
        resolve_command_block(
            &mut json!({"id":"A","check":{"kind":"command","command":"run","command_block":false}}),
            ""
        ),
        None
    );
    let inline = r#"```json
{"id":"A","check":{"kind":"command","command":"printf \\\"ok\\\"\\n"}}
```"#;
    let inline_text = extract_object(inline);
    let inline_entry = reply_entry_with_blocks(inline, "A").unwrap();
    assert_eq!(inline_entry, reply_entry(&inline_text, "A").unwrap());
}

#[test]
fn recorded_live_replies_use_the_shared_expected_outcomes() {
    let fixtures: Vec<ValueFixture> =
        serde_json::from_str(include_str!("fixtures/decompose_live_reply_outcomes.json")).unwrap();
    for fixture in fixtures {
        match fixture.id.as_str() {
            "SUP-REQ-AHDM-022" => {
                assert!(
                    reply_entry_with_blocks(&fixture.reply, &fixture.id).is_some(),
                    "{}",
                    fixture.expected
                );
            }
            "SUP-REQ-AHDM-020" => {
                assert!(reply_entry_with_blocks(&fixture.reply, &fixture.id).is_none());
                assert!(fixture.expected.contains("malformed inline JSON escape"));
            }
            "SUP-REQ-AHDM-021" => {
                assert!(reply_entry_with_blocks(&fixture.reply, &fixture.id).is_none());
                assert!(fixture.expected.contains("more than one entry"));
            }
            "SUP-REQ-BT-001" => {
                assert!(
                    reply_entry_with_blocks(&fixture.reply, &fixture.id).is_none(),
                    "{}",
                    fixture.expected
                );
            }
            "live-prose-after-json-close" | "live-close-and-open-same-line" => {
                assert!(reply_entry_with_blocks(&fixture.reply, &fixture.id).is_none());
                assert!(
                    fixture
                        .expected
                        .contains("check.command_block is true but no check block is present")
                );
            }
            "corrected-fence-layout" => {
                assert!(reply_entry_with_blocks(&fixture.reply, &fixture.id).is_some());
                assert_eq!(fixture.expected, "accepted");
            }
            "floor-command-block-flag" | "floor-command-block-fence" => {
                let text = extract_object(&fixture.reply);
                let mut entry = reply_entry(&text, "A").unwrap();
                let refusal = resolve_command_block(&mut entry, &fixture.reply).unwrap();
                assert_eq!(fixture.expected, format!("refuse: {refusal}"));
                assert!(reply_entry_with_blocks(&fixture.reply, "A").is_none());
            }
            other => panic!("unexpected live fixture {other}"),
        }
    }
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
