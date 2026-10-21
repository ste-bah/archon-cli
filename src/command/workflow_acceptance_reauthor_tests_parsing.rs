use super::*;

#[test]
fn escalation_requires_route_diagnostics_for_every_pending_check_and_reports_them() {
    let pending = ids(&["AC-F-001", "AC-F-002"]);
    let lines = vec![
        baseline_finding(
            "AC-F-001",
            "state.txt (generated or recreated by the check's own run)",
        ),
        baseline_finding(
            "AC-F-002",
            "../source.rs (sibling tree seen from this root, not movable)",
        ),
    ];
    let escalation = super::escalation::baseline_escalation(&pending, &lines, 6)
        .expect("complete route on all checks");
    assert!(
        escalation.contains("repair route 6 time(s)"),
        "{escalation}"
    );
    assert!(
        escalation.contains("Last exclusion reasons:"),
        "{escalation}"
    );
    assert!(escalation.contains("generated or recreated by the check's own run"));
    assert!(escalation.contains("sibling tree seen from this root, not movable"));
    assert!(!escalation.contains("restate"));
    assert!(!escalation.contains("drop it"));

    let incomplete = vec![
        lines[0].clone(),
        "check 'AC-F-002': it passed before implementation".into(),
    ];
    assert!(super::escalation::baseline_escalation(&pending, &incomplete, 6).is_none());
}

#[test]
fn acceptance_reauthor_accepts_shared_check_blocks_and_refuses_malformed_block_forms() {
    let set = frozen_set(&[("AC-F-001", "jq -e '.a == true' out.json", false)]);
    let contract = set.contract();
    let frozen = &contract.acceptance[0];
    let block_reply =
        |check: &str, body: &str| format!("{{\"id\":\"AC-F-001\",\"check\":{check}}}{body}");
    let block = block_reply(
        r#"{"kind":"command","command_block":true,"cwd":"project_root"}"#,
        "\n```check\njq -e '.a == true and .b == true' out.json\n```",
    );
    let inline = r#"{"id":"AC-F-001","check":{"kind":"command","command":"jq -e '.a == true and .b == true' out.json","cwd":"project_root"}}"#;
    let from_block = candidate_entry(&block, frozen).expect("check block is resolved");
    let from_inline = candidate_entry(inline, frozen).expect("inline command remains accepted");
    assert_eq!(from_block.check, from_inline.check);
    let unclosed_check_with_json = format!(
        "```json\n{{\"id\":\"AC-F-001\",\"check\":{{\"kind\":\"command\",\"command_block\":true,\"cwd\":\"project_root\"}}}}\n```\n```check\nrun"
    );
    let expected_refusal = "the ```check block opened at line 4 is never closed (the reply may have been cut off); close every block with a line that is exactly ```";
    assert_eq!(
        candidate_entry(&unclosed_check_with_json, frozen).unwrap_err(),
        expected_refusal
    );
    let fenced_json = block.replace(
        "{\"id\":\"AC-F-001\",\"check\":",
        "```json\n{\"id\":\"AC-F-001\",\"check\":",
    );
    let fenced_json = fenced_json.replace("}\n```check", "}\n```\n```check");
    assert_eq!(
        candidate_entry(&fenced_json, frozen)
            .expect("fenced JSON and check block are resolved")
            .check,
        from_inline.check
    );

    let cases = [
        (
            block_reply(
                r#"{"kind":"command","command_block":true,"cwd":"project_root"}"#,
                "\n```check\nrun",
            ),
            "never closed",
        ),
        (
            block_reply(
                r#"{"kind":"command","command_block":true,"cwd":"project_root"}"#,
                "\n```check\na\n```\n```check\nb\n```",
            ),
            "more than one check block",
        ),
        (
            block_reply(
                r#"{"kind":"command","command":"inline","command_block":true,"cwd":"project_root"}"#,
                "\n```check\nrun\n```",
            ),
            "remove check.command from the JSON when you use the check block",
        ),
        (
            block_reply(
                r#"{"kind":"command","command_block":true,"cwd":"project_root"}"#,
                "",
            ),
            "the fence info string must be exactly 'check'; found fence labels: none",
        ),
    ];
    for (reply, expected) in cases {
        let error = candidate_entry(&reply, frozen).expect_err("malformed block is refused");
        assert!(
            error.contains(expected),
            "expected {expected:?}, got {error:?}"
        );
    }
}

#[test]
fn acceptance_reauthor_prompt_layout_is_accepted_by_the_parser() {
    let set = frozen_set(&[("AC-F-001", "jq -e '.a == true' out.json", false)]);
    let contract = set.contract();
    let frozen = &contract.acceptance[0];
    let reply = concat!(
        "```json\n",
        "{\"id\":\"AC-F-001\",\"check\":{\"kind\":\"command\",\"command_block\":true,\"cwd\":\"project_root\"}}\n",
        "```\n",
        "```check\n",
        "jq -e '.a == true and .b == true' out.json\n",
        "```"
    );
    assert!(candidate_entry(reply, frozen).is_ok());
}

#[tokio::test]
async fn acceptance_reauthor_prompts_do_not_ban_their_check_fence() {
    let set = frozen_set(&[("AC-F-001", "jq -e '.a == true' out.json", false)]);
    let client = ScriptedAuthorJudge::new(
        |entry, _| command_entry(entry, "jq -e '.a == true and .b == true' out.json"),
        |_, _| true,
    );
    let before = set.contract();
    let _ = reauthor(
        &client,
        &before,
        &ids(&["AC-F-001"]),
        &scope(&set),
        "sonnet",
        &set.gate(),
    )
    .await
    .expect("the corrected entry is accepted");
    let reauthor_prompt = client.prompts.lock().unwrap()[0].clone();
    let decomposition_prompt = include_str!("workflow_decompose_v1_acceptance.js");
    for prompt in [&reauthor_prompt, decomposition_prompt] {
        assert!(prompt.contains("followed by exactly one non-empty fence labelled `check`"));
        assert!(prompt.contains("Emit no prose, explanations, headings or other fences."));
        assert!(!prompt.contains("no Markdown code fences"));
    }
}

#[test]
fn malformed_json_reply_explains_the_escape_at_its_location() {
    let set = frozen_set(&[("AC-F-001", "run", false)]);
    let contract = set.contract();
    let frozen = &contract.acceptance[0];
    let reply = r#"{"id":"AC-F-001","label":"bad\    if"}"#;
    let error = candidate_entry(reply, frozen).expect_err("invalid escape is refused");
    assert!(
        error.starts_with("the reply for check 'AC-F-001' is not one JSON entry ("),
        "{error}"
    );
    assert!(error.contains("Syntax at line 1 column"), "{error}");
    assert!(error.contains("<HERE>"), "{error}");
    assert!(
        error.contains("literal backslash inside a JSON string must be written as `\\\\`"),
        "{error}"
    );
}

#[test]
fn live_fenced_replies_report_locations_inside_the_json_entry() {
    let set = frozen_set(&[("AC-F-001", "run", false)]);
    let contract = set.contract();
    let frozen = &contract.acceptance[0];
    for (fixture, expected) in [
        (
            include_str!("../../tests/fixtures/reply-parser/reply-one.txt"),
            "Syntax at line 2 column 631",
        ),
        (
            include_str!("../../tests/fixtures/reply-parser/reply-two.txt"),
            "Syntax at line 2 column 6520",
        ),
    ] {
        let error = candidate_entry(fixture, frozen).expect_err("fixture contains malformed JSON");
        assert!(error.contains(expected), "{error}");
        assert!(error.contains("<HERE>"), "{error}");
    }
    let first = include_str!("../../tests/fixtures/reply-parser/reply-one.txt");
    let error = candidate_entry(first, frozen).expect_err("invalid escape is refused");
    assert!(
        error.contains("the check is a multi-line script; return it in block form instead:"),
        "{error}"
    );
}

#[test]
fn reply_shape_diagnoses_are_independent_and_valid_entries_still_parse() {
    let set = frozen_set(&[("AC-X-001", "run", false)]);
    let contract = set.contract();
    let frozen = &contract.acceptance[0];
    let reply = include_str!("../../tests/fixtures/reply-parser/reply-three.txt");
    let extraction = crate::command::workflow_task_set::acceptance_reply_blocks::extract_object_outcome_with_markers(reply, &[]);
    let diagnoses = super::reply_parse::shape_diagnoses(reply, &[], &extraction);
    for diagnosis in [
        "return the entry object itself, not an array",
        "return the entry only",
        "the check fence is empty; put the script in it",
    ] {
        assert!(
            diagnoses.iter().any(|item| item == diagnosis),
            "missing {diagnosis:?}: {diagnoses:?}"
        );
    }
    let legacy_result =
        candidate_entry(reply, frozen).expect_err("the reply keeps the legacy parse result");
    assert!(
        !legacy_result.contains("line 1 column 1"),
        "{legacy_result}"
    );

    let array_only = format!(
        "[{valid}]",
        valid = r#"{"id":"AC-X-001","check":{"kind":"command","command":"run another check","cwd":"project_root"}}"#
    );
    let extracted_array_entry =
        crate::command::workflow_task_set::acceptance_reply_blocks::extract_object(&array_only);
    assert_eq!(
        extracted_array_entry,
        r#"{"id":"AC-X-001","check":{"kind":"command","command":"run another check","cwd":"project_root"}}"#
    );
    let legacy_array_error = candidate_entry(&array_only, frozen)
        .expect_err("the original document parser still handles a bare array as before");
    assert!(
        !legacy_array_error.contains("formatting problems"),
        "{legacy_array_error}"
    );

    let two_entries = concat!(
        r#"{"id":"AC-X-001","check":{"kind":"command","command":"first","cwd":"project_root"}}"#,
        "\n",
        r#"{"id":"AC-X-001","check":{"kind":"command","command":"second","cwd":"project_root"}}"#
    );
    assert!(
        candidate_entry(two_entries, frozen)
            .unwrap_err()
            .contains("the reply holds 2 different entries; return exactly one")
    );

    let empty_check = concat!(
        r#"{"id":"AC-X-001","check":{"kind":"command","command":"run","cwd":"project_root"}}"#,
        "\n```check\n```"
    );
    assert_eq!(
        candidate_entry(empty_check, frozen).unwrap_err(),
        "the reply mixes an inline command with a check fence and must use one form"
    );

    let valid = r#"{"id":"AC-X-001","check":{"kind":"command","command":"run another check","cwd":"project_root"}}"#;
    assert!(candidate_entry(valid, frozen).is_ok());
    let fenced_with_prose_before = format!("A revised entry follows.\n```json\n{valid}\n```");
    let parsed_before = candidate_entry(&fenced_with_prose_before, frozen)
        .expect("prose before the JSON fence was accepted by the original extractor");
    assert!(matches!(
        &parsed_before.check,
        AcceptanceCheck::Command { command, .. } if command == "run another check"
    ));
    let fenced_with_prose_after = format!("```json\n{valid}\n```\nThe entry is complete.");
    let parsed_after = candidate_entry(&fenced_with_prose_after, frozen)
        .expect("prose after the JSON fence was accepted by the original extractor");
    assert!(matches!(
        &parsed_after.check,
        AcceptanceCheck::Command { command, .. } if command == "run another check"
    ));
    let valid_block = concat!(
        "```json\n",
        r#"{"id":"AC-X-001","check":{"kind":"command","cwd":"project_root","command_block":true}}"#,
        "\n```\n```check\nrun another check\n```"
    );
    assert!(candidate_entry(valid_block, frozen).is_ok());
}

#[test]
fn inline_command_and_check_fence_have_one_shared_refusal() {
    let set = frozen_set(&[("AC-F-001", "run", false)]);
    let contract = set.contract();
    let frozen = &contract.acceptance[0];
    let reply = concat!(
        "```json\n",
        r#"{"id":"AC-F-001","check":{"kind":"command","command":"run","cwd":"project_root"}}"#,
        "\n```\n```check\nrun\n```"
    );
    assert_eq!(
        candidate_entry(reply, frozen).unwrap_err(),
        "acceptance entry AC-F-001 mixes inline check.command and a check fence; for a multi-line script, use block form with check.command_block true, no inline check.command, and the script in the check fence"
    );
}

#[test]
fn command_block_accepts_absent_command_and_refuses_empty_command() {
    let set = frozen_set(&[("AC-F-001", "old", false)]);
    let contract = set.contract();
    let frozen = &contract.acceptance[0];
    let reply = concat!(
        "```json\n",
        r#"{"id":"AC-F-001","check":{"kind":"command","cwd":"project_root","command_block":true}}"#,
        "\n```\n```check\nrun\n```"
    );
    let entry =
        candidate_entry(reply, frozen).expect("block form with no inline command is accepted");
    let AcceptanceCheck::Command { command, .. } = entry.check else {
        panic!("expected a command check");
    };
    assert_eq!(command, "run");

    let empty_command_reply = concat!(
        "```json\n",
        r#"{"id":"AC-F-001","check":{"kind":"command","cwd":"project_root","command":"","command_block":true}}"#,
        "\n```\n```check\nrun\n```"
    );
    assert_eq!(
        candidate_entry(empty_command_reply, frozen).unwrap_err(),
        "acceptance entry AC-F-001 returned both check.command and check.command_block — remove check.command from the JSON when you use the check block"
    );
}
