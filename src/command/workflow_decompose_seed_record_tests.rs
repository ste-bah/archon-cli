use super::{
    extract_object, extract_object_with_markers, reply_entry, reply_entry_with_blocks,
    resolve_command_block, resolve_command_block_with_markers, unclosed_block_refusal,
    unclosed_block_refusal_with_markers,
};
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
fn check_block_line_endings_are_normalized_and_match_the_javascript_resolver() {
    let cases = [
        ("```check\r\nfirst\r\nsecond\r\n```", "first\nsecond"),
        ("```check\nfirst\rsecond\r```", "first\nsecond"),
        (
            "```check\r\nfirst\nsecond\rthird\n```",
            "first\nsecond\nthird",
        ),
    ];
    for (reply, expected) in cases {
        let mut entry = json!({"id":"A","check":{"kind":"command","command_block":true}});
        assert_eq!(resolve_command_block(&mut entry, reply), None);
        assert_eq!(entry["check"]["command"], expected);
    }

    let reply = "```json\r\n{\"id\":\"A\",\"check\":{\"kind\":\"command\",\"command_block\":true}}\r\n```\r\n```check\r\nfirst\r\nsecond\r\n```";
    let mut entry = json!({"id":"A","check":{"kind":"command","command_block":true}});
    assert_eq!(resolve_command_block(&mut entry, reply), None);
    let js_source = include_str!("workflow_decompose_reply_blocks.js");
    let script = format!(
        "{js_source}\nconst reply = {}; const entry = {{id:'A',check:{{kind:'command',command_block:true}}}}; const refusal = resolveCommandBlock(entry, reply); if (refusal) throw new Error(refusal); process.stdout.write(entry.check.command);",
        serde_json::to_string(reply).unwrap()
    );
    let output = std::process::Command::new("node")
        .arg("-e")
        .arg(script)
        .output()
        .expect("node must be available for the JavaScript resolver parity check");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        entry["check"]["command"],
        String::from_utf8_lossy(&output.stdout).as_ref()
    );
}

#[test]
fn check_block_refusals_and_inline_command_match_contract() {
    let unclosed = "```check\nrun\n{\"id\":\"A\"}";
    assert_eq!(extract_object(unclosed), "");
    assert_eq!(
        resolve_command_block(
            &mut json!({"id":"A","check":{"kind":"command","command_block":true}}),
            unclosed
        ),
        Some("the ```check block opened at line 1 is never closed (the reply may have been cut off); close every block with a line that is exactly ```".into())
    );
    assert_eq!(
        resolve_command_block(
            &mut json!({"id":"A","check":{"kind":"command","command_block":true}}),
            "{}"
        ),
        Some("acceptance entry A check.command_block is true but no check block is present — the fence info string must be exactly 'check'; found fence labels: none".into())
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
    let mut inferred = json!({"id":"A","check":{"kind":"command","cwd":"project_root"}});
    assert_eq!(
        resolve_command_block(&mut inferred, "```check\nrun\n```"),
        None
    );
    assert_eq!(inferred["check"]["command"], "run");
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
fn live_midline_fence_diagnosis_and_marker_removal_match_javascript() {
    let marker = "</think>".to_string();
    let mut lines = vec![format!(
        "{{\"id\":\"A\",\"check\":{{\"kind\":\"command\",\"command_block\":true}}}}{marker}```check"
    )];
    lines.extend((1..=73).map(|line| format!("script line {line}")));
    lines.push("```".into());
    let reply = lines.join("\n");
    assert_eq!(reply.lines().count(), 75);
    let refusal = unclosed_block_refusal(&reply).unwrap();
    assert!(refusal.starts_with("fence marker on line 1 follows text; a fence must start its own line; text before the marker: "));
    assert!(refusal.contains("\\\"id\\\":\\\"A\\\""));
    assert!(!refusal.contains("may have been cut off"));

    let object = extract_object_with_markers(&reply, std::slice::from_ref(&marker));
    let mut entry: serde_json::Value = serde_json::from_str(&object).unwrap();
    assert_eq!(
        resolve_command_block_with_markers(&mut entry, &reply, std::slice::from_ref(&marker)),
        None
    );
    let expected = (1..=73)
        .map(|line| format!("script line {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(entry["check"]["command"], expected);

    let js_source = include_str!("workflow_decompose_reply_blocks.js");
    let script = format!(
        "{js_source}\nconst reply = {}; const markers = {}; let diagnostic = ''; try {{ extractJsonObject(reply); }} catch (error) {{ diagnostic = error.message; }} const entry = JSON.parse(extractJsonObject(reply, markers)); const refusal = resolveCommandBlock(entry, reply, markers); if (refusal) throw new Error(refusal); process.stdout.write(JSON.stringify({{command:entry.check.command, diagnostic}}));",
        serde_json::to_string(&reply).unwrap(),
        serde_json::to_string(&[marker]).unwrap()
    );
    let output = std::process::Command::new("node")
        .arg("-e")
        .arg(script)
        .output()
        .expect("node must be available for the JavaScript resolver parity check");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let parity: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(parity["command"], expected);
    assert_eq!(parity["diagnostic"], refusal);

    let cut_off = "{\"id\":\"A\"}\n```check\nscript";
    assert!(
        unclosed_block_refusal(cut_off)
            .unwrap()
            .contains("may have been cut off")
    );
    assert_eq!(
        unclosed_block_refusal("```json\n{}\n```\n```").unwrap(),
        "closing fence on line 4 has no opener; every closing fence must close an open block"
    );
}

#[test]
fn recorded_live_replies_use_the_shared_expected_outcomes_reply_blocks() {
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
            "unicode-fence-label" => {
                let text = extract_object(&fixture.reply);
                let mut entry = reply_entry(&text, &fixture.id).unwrap();
                let refusal = resolve_command_block(&mut entry, &fixture.reply).unwrap();
                assert_eq!(refusal, fixture.expected.strip_prefix("refuse: ").unwrap());
            }
            "corrected-fence-layout" => {
                assert!(reply_entry_with_blocks(&fixture.reply, &fixture.id).is_some());
                assert_eq!(fixture.expected, "accepted");
            }
            "single-check-block-implicit-command-block" => {
                let entry = reply_entry_with_blocks(&fixture.reply, &fixture.id).unwrap();
                assert_eq!(entry["check"]["command"], "run focused check");
                assert_eq!(fixture.expected, "accepted");
            }
            "false-command-block-with-check-fence"
            | "command-and-command-block-with-check-fence"
            | "multiple-check-fences"
            | "empty-check-fence"
            | "check-fence-on-floor-kind" => {
                let text = extract_object(&fixture.reply);
                let mut entry = reply_entry(&text, &fixture.id).unwrap();
                let refusal = resolve_command_block(&mut entry, &fixture.reply).unwrap();
                assert_eq!(fixture.expected, format!("refuse: {refusal}"));
                assert!(reply_entry_with_blocks(&fixture.reply, &fixture.id).is_none());
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

#[path = "evidence_423_line_endings_tests.rs"]
mod evidence_423;

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

#[test]
fn configured_reply_markers_are_removed_outside_fences_and_preserved_inside_with_javascript_parity()
{
    let markers = vec!["<LEAK_A>".to_string(), "<LEAK_B>".to_string()];
    let reply = "<LEAK_A><LEAK_B>\r\n{\"id\":\"A\",\"check\":{\"kind\":\"command\",\"command_block\":true}}<LEAK_A><LEAK_B>```check\r\necho \"<LEAK_A><LEAK_B>\"\r\n```\r\n<LEAK_B>tail";
    let json = extract_object_with_markers(reply, &markers);
    assert_eq!(
        json,
        "{\"id\":\"A\",\"check\":{\"kind\":\"command\",\"command_block\":true}}"
    );
    let mut entry = json!( {"id":"A","check":{"kind":"command","command_block":true}} );
    assert_eq!(
        resolve_command_block_with_markers(&mut entry, reply, &markers),
        None
    );
    assert_eq!(entry["check"]["command"], "echo \"<LEAK_A><LEAK_B>\"");
    assert!(
        unclosed_block_refusal_with_markers("<LEAK_A>```check\r\necho x", &markers)
            .unwrap()
            .contains("the reply may have been cut off")
    );

    let js_source = include_str!("workflow_decompose_reply_blocks.js");
    let script = format!(
        "{js_source}\nconst reply = {}; const markers = {}; const json = extractJsonObject(reply, markers); const entry = {{id:'A',check:{{kind:'command',command_block:true}}}}; const refusal = resolveCommandBlock(entry, reply, markers); if (refusal) throw new Error(refusal); let cutoff = ''; try {{ replyFenceBlocks('```check\\r\\necho x',markers); }} catch (error) {{ cutoff = error.message; }} process.stdout.write(JSON.stringify({{json,command:entry.check.command,cutoff}}));",
        serde_json::to_string(reply).unwrap(),
        serde_json::to_string(&markers).unwrap()
    );
    let output = std::process::Command::new("node")
        .arg("-e")
        .arg(script)
        .output()
        .expect("node must be available for the JavaScript resolver parity check");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let js: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(js["json"], json);
    assert_eq!(js["command"], entry["check"]["command"]);
    assert!(js["cutoff"].as_str().unwrap().contains("the reply may have been cut off"));
}

#[test]
fn midline_fence_boundary_requires_the_first_complete_json_object() {
    let marker = "<LEAK_A>".to_string();
    let markers = std::slice::from_ref(&marker);
    let json = r#"{"id":"A","note":"a}b","nested":{"value":{"ok":true}},"check":{"kind":"command","command_block":true}}"#;
    let valid = format!("{json}{marker}```check\necho ok\n```");
    assert_eq!(extract_object_with_markers(&valid, markers), json);
    assert!(unclosed_block_refusal_with_markers(&valid, markers).is_none());

    let prose =
        format!("{{\"id\":\"A\",\"nested\":{{\"ok\":true}}}} prose }}{marker}```check\nbody\n```");
    let refusal = unclosed_block_refusal_with_markers(&prose, markers).unwrap();
    assert!(
        refusal
            .starts_with("fence marker on line 1 follows text; a fence must start its own line;")
    );
    assert!(
        unclosed_block_refusal_with_markers(&valid, &[])
            .unwrap()
            .starts_with("fence marker on line 1 follows text; a fence must start its own line;")
    );
}
