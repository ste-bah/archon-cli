use super::candidate_entry;
use crate::command::workflow_task_set::republish::test_fixture::frozen_set_proven as frozen_set;
use archon_workflow::task_set_contract::AcceptanceCheck;

#[test]
fn acceptance_reauthor_reports_info_string_fences_that_cannot_close_json() {
    let set = frozen_set(&[("AC-F-001", "run", false)]);
    let frozen = &set.contract().acceptance[0];
    let malformed = concat!(
        "```json\n",
        "{\"id\":\"AC-F-001\",\"check\":{\"kind\":\"command\",\"command_block\":true,\"cwd\":\"project_root\"}}\n",
        "```check\n",
        "echo ok\n",
    );
    assert_eq!(
        candidate_entry(malformed, frozen).unwrap_err(),
        "the fence opened on line 1 is not closed; line 3 \"```check\" has an info string and cannot close it; put a bare ``` on its own line before the next fence; keep exactly one non-empty check fence for block form"
    );

    let inline = r#"{"id":"AC-F-001","check":{"kind":"command","command":"run"}}"#;
    let refusal = candidate_entry(&format!("{inline}\n```check\nrun\n```"), frozen).unwrap_err();
    assert!(
        refusal
            .contains("the reply mixes an inline command with a check fence and must use one form"),
        "{refusal}"
    );
}

#[test]
fn resolved_check_script_may_contain_info_fence_lines_and_is_preserved() {
    let set = frozen_set(&[("AC-F-001", "run", false)]);
    let frozen = &set.contract().acceptance[0];
    let reply = concat!(
        "{\"id\":\"AC-F-001\",\"check\":{\"kind\":\"command\",\"command_block\":true,\"cwd\":\"project_root\"}}\n",
        "```check\n",
        "cat <<'EOF'\n",
        "```js\n",
        "{\"example\": true}\n",
        "EOF\n",
        "```"
    );
    let parsed = candidate_entry(reply, frozen).expect("resolved script fences are script content");
    let AcceptanceCheck::Command { command, .. } = parsed.check else {
        panic!("expected command check")
    };
    assert_eq!(command, "cat <<'EOF'\n```js\n{\"example\": true}\nEOF");
}

#[test]
fn malformed_json_with_valid_check_fence_reports_entry_syntax_and_block_form() {
    let set = frozen_set(&[("AC-F-001", "run", false)]);
    let frozen = &set.contract().acceptance[0];
    let reply = concat!(
        "{\"id\":\"AC-F-001\",\"check\":{\"kind\":\"command\",\"command\":\"run\",}}\n",
        "```check\n",
        "run\n",
        "```"
    );
    let refusal = candidate_entry(reply, frozen).unwrap_err();
    assert!(refusal.contains("Syntax at line 1 column"), "{refusal}");
    assert!(
        refusal.contains("keep exactly one non-empty check fence for block form"),
        "{refusal}"
    );
    assert!(
        !refusal.contains("return the raw JSON object alone"),
        "{refusal}"
    );
}

#[test]
fn malformed_multiline_inline_command_ends_with_block_form_direction() {
    let set = frozen_set(&[("AC-F-001", "run", false)]);
    let frozen = &set.contract().acceptance[0];
    let reply =
        r#"{"id":"AC-F-001","check":{"kind":"command","command":"set -eu\nprintf x\necho \q"}}"#;
    let finding = candidate_entry(reply, frozen).unwrap_err();
    assert!(
        finding.contains("the check is a multi-line script; return it in block form instead:"),
        "{finding}"
    );
    assert!(finding.ends_with("then no escaping is needed"), "{finding}");
    assert!(!finding.contains("raw JSON object alone"), "{finding}");
}

#[test]
fn malformed_single_line_inline_json_keeps_raw_json_advice() {
    let set = frozen_set(&[("AC-F-001", "run", false)]);
    let frozen = &set.contract().acceptance[0];
    let reply = r#"{"id":"AC-F-001","check":{"kind":"command","command":"run",}}"#;
    let finding = candidate_entry(reply, frozen).unwrap_err();
    assert!(finding.contains("Syntax at line 1 column"), "{finding}");
    assert!(
        finding.ends_with("return the raw JSON object alone"),
        "{finding}"
    );
    assert!(!finding.contains("block form instead"), "{finding}");
}

#[test]
fn malformed_visible_block_form_gets_check_fence_advice_without_being_accepted() {
    let set = frozen_set(&[("AC-F-001", "run", false)]);
    let frozen = &set.contract().acceptance[0];
    let reply = r#"{"id":"AC-F-001","check":{"kind":"command","command_block":true,}}"#;
    let finding = candidate_entry(reply, frozen).unwrap_err();
    assert!(finding.ends_with(
        "correct the JSON entry and put the script in exactly one non-empty fence labelled check after it"
    ), "{finding}");
    assert!(
        !finding.contains("return the raw JSON object alone"),
        "{finding}"
    );
}

#[test]
fn malformed_command_string_does_not_create_visible_block_form_intent() {
    let set = frozen_set(&[("AC-F-001", "run", false)]);
    let frozen = &set.contract().acceptance[0];
    let reply = r#"{"id":"AC-F-001","check":{"kind":"command","command":"echo \"command_block\": true, nope",}}"#;
    let finding = candidate_entry(reply, frozen).unwrap_err();
    assert!(
        finding.ends_with("return the raw JSON object alone"),
        "{finding}"
    );
    assert!(
        !finding.contains("put the script in exactly one non-empty fence"),
        "{finding}"
    );
}

#[test]
fn malformed_true_object_key_gets_block_form_advice() {
    let set = frozen_set(&[("AC-F-001", "run", false)]);
    let frozen = &set.contract().acceptance[0];
    let reply = r#"{"id":"AC-F-001","check":{"kind":"command","command_block": true,}}"#;
    let finding = candidate_entry(reply, frozen).unwrap_err();
    assert!(finding.ends_with(
        "correct the JSON entry and put the script in exactly one non-empty fence labelled check after it"
    ), "{finding}");
    assert!(
        !finding.contains("return the raw JSON object alone"),
        "{finding}"
    );
}

#[test]
fn nested_command_block_key_does_not_create_visible_block_form_intent() {
    let set = frozen_set(&[("AC-F-001", "run", false)]);
    let frozen = &set.contract().acceptance[0];
    let nested = r#"{"id":"AC-F-001","metadata":{"command_block":true},"check":{"kind":"command","command":"run",}}"#;
    let finding = candidate_entry(nested, frozen).unwrap_err();
    assert!(
        finding.ends_with("return the raw JSON object alone"),
        "{finding}"
    );

    let direct = r#"{"id":"AC-F-001","check":{"kind":"command","command_block":true,}}"#;
    let finding = candidate_entry(direct, frozen).unwrap_err();
    assert!(finding.ends_with(
        "correct the JSON entry and put the script in exactly one non-empty fence labelled check after it"
    ), "{finding}");
}

#[test]
fn live_duplicate_check_openers_match_the_rust_diagnostic_text() {
    let set = frozen_set(&[("AC-F-001", "run", false)]);
    let frozen = &set.contract().acceptance[0];
    let reply = concat!(
        "```json\n",
        "{\"id\":\"AC-F-001\",\"check\":{\"kind\":\"command\",\"command_block\":true}}\n",
        "```check\n",
        "```check\n",
        "```"
    );
    assert_eq!(
        candidate_entry(reply, frozen).unwrap_err(),
        "the fence opened on line 1 is not closed; line 3 \"```check\" has an info string and cannot close it; line 4 is a duplicate check fence opener; put a bare ``` on its own line before the next fence; keep exactly one non-empty check fence for block form"
    );
}
