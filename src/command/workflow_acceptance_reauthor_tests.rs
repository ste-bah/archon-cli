//! The bounded re-author loop over a scripted author and judge.

use super::test_client::{ScriptedAuthorJudge, command_entry, resolve_in_bare_project};
use super::*;
use crate::command::workflow_task_set::republish::test_fixture::frozen_set_proven as frozen_set;

fn scope(
    set: &crate::command::workflow_task_set::republish::test_fixture::FrozenSet,
) -> AuthorScope {
    AuthorScope::for_task_set(set.project.path(), &set.tasks, &set.prd)
        .expect("the task set's repository record is believed")
}

fn ids(list: &[&str]) -> BTreeSet<String> {
    list.iter().map(|id| id.to_string()).collect()
}

fn baseline_finding(id: &str, candidates: &str) -> String {
    format!(
        "check '{id}': it {} at base; data the check writes itself is never evidence; movable inputs must exist under the check's selected root before the run; to guard behaviour implemented in code, select the root that contains the existing implementation or test dependency and name it relative to that root; the check must still fail on its own assertion when that input is moved aside; considered inputs: {candidates}",
        super::executability::CANNOT_FAIL
    )
}

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

#[tokio::test]
async fn a_refuted_check_is_reauthored_and_rejudged_and_nothing_else_changes() {
    let set = frozen_set(&[
        ("AC-F-001", "jq -e '.a == true' out.json", true),
        ("AC-F-002", "jq -e '.b == true' out.json", false),
    ]);
    let client = ScriptedAuthorJudge::new(
        |entry, _| command_entry(entry, "jq -e '.b == true and .c == true' out.json"),
        |_, _| true,
    );
    let before = set.contract();
    let after = reauthor(
        &client,
        &before,
        &ids(&["AC-F-002"]),
        &scope(&set),
        "sonnet",
        &set.gate(),
    )
    .await
    .expect("an accepted re-author");
    assert_eq!(
        after.acceptance[0], before.acceptance[0],
        "unnamed entry untouched"
    );
    let repaired = &after.acceptance[1];
    assert_eq!(repaired.judgment.verdict, JudgeDecision::Accepted);
    assert_eq!(
        repaired.criterion, before.acceptance[1].criterion,
        "criterion is host-owned"
    );
    assert_eq!(
        repaired.check,
        AcceptanceCheck::Command {
            command: "jq -e '.b == true and .c == true' out.json".into(),
            cwd: archon_workflow::task_set_contract::TrustedCwd::ProjectRoot,
        }
    );
    assert!(
        repaired.judgment.sampling.is_some(),
        "the judge's sampling is recorded"
    );
    assert_eq!(client.authored(), 1);
    assert_eq!(
        *client.judged_ids.lock().unwrap(),
        vec!["AC-F-002".to_string(); 2]
    );
}

#[tokio::test]
async fn a_check_the_judge_keeps_refuting_fails_after_the_bound_with_a_per_check_report() {
    let set = frozen_set(&[("AC-F-001", "jq -e '.a == true' out.json", false)]);
    let client = ScriptedAuthorJudge::new(
        |entry, attempt| command_entry(entry, &format!("jq -e '.a{attempt} == true' out.json")),
        |_, _| false,
    );
    let error = reauthor(
        &client,
        &set.contract(),
        &ids(&["AC-F-001"]),
        &scope(&set),
        "sonnet",
        &set.gate(),
    )
    .await
    .expect_err("never accepted")
    .to_string();
    assert_eq!(client.authored(), REAUTHOR_ATTEMPTS);
    assert!(error.contains("AC-F-001"), "{error}");
    assert!(error.contains("after 3 re-author attempt(s)"), "{error}");
    assert!(error.contains("the check misses a branch"), "{error}");
}

/// The loop is bounded by progress, not by a fixed count: a check repaired
/// on attempt 3 resets the idle count, so another repaired on attempt 5 is
/// still reached; a check nobody repairs ends it after
/// `REAUTHOR_ATTEMPTS` idle attempts, and the report says how many ran.
#[tokio::test]
async fn the_reauthor_runs_while_it_makes_progress_and_stops_when_it_makes_none() {
    let set = frozen_set(&[
        ("AC-F-001", "jq -e '.a == true' out.json", false),
        ("AC-F-002", "jq -e '.b == true' out.json", false),
    ]);
    let author = |entry: &serde_json::Value, attempt: usize| {
        command_entry(entry, &format!("jq -e '.v{attempt} == true' out.json"))
    };
    let accepts = |id: &str, check: &serde_json::Value| {
        let command = check["command"].as_str().unwrap_or_default();
        (id == "AC-F-001" && command.contains(".v3 "))
            || (id == "AC-F-002" && command.contains(".v5 "))
    };
    let client = ScriptedAuthorJudge::new(author, accepts);
    let after = reauthor(
        &client,
        &set.contract(),
        &ids(&["AC-F-001", "AC-F-002"]),
        &scope(&set),
        "sonnet",
        &set.gate(),
    )
    .await
    .expect("progress on attempt 3 keeps the loop going to attempt 5");
    assert_eq!(
        client.authored(),
        3 + 5,
        "each pending check is authored per attempt"
    );
    assert!(
        (after.acceptance.iter()).all(|entry| entry.judgment.verdict == JudgeDecision::Accepted)
    );
    assert!(
        client.prompts.lock().unwrap()[0].contains("consecutive attempts that repair no check"),
        "the author is told the bound"
    );

    let never = ScriptedAuthorJudge::new(author, move |id, check| {
        id == "AC-F-001" && accepts(id, check)
    });
    let error = reauthor(
        &never,
        &set.contract(),
        &ids(&["AC-F-001", "AC-F-002"]),
        &scope(&set),
        "sonnet",
        &set.gate(),
    )
    .await
    .expect_err("AC-F-002 is never repaired")
    .to_string();
    assert_eq!(
        never.authored(),
        3 + 6,
        "three idle attempts after the last repair"
    );
    assert!(
        error.contains("stopped after 6 attempt(s): the last 3 repaired no named check"),
        "{error}"
    );
    assert!(
        error.contains("AC-F-002") && !error.contains("check 'AC-F-001'"),
        "{error}"
    );
}

#[tokio::test]
async fn a_reply_repeating_the_refuted_check_is_never_judged_and_costs_its_attempt() {
    let set = frozen_set(&[("AC-F-001", "jq -e '.a == true' out.json", false)]);
    let client = ScriptedAuthorJudge::new(
        |entry, _| command_entry(entry, "jq -e '.a == true' out.json"),
        |_, _| true,
    );
    let error = reauthor(
        &client,
        &set.contract(),
        &ids(&["AC-F-001"]),
        &scope(&set),
        "sonnet",
        &set.gate(),
    )
    .await
    .expect_err("a repeated check is not a repair")
    .to_string();
    assert!(error.contains("repeats the check"), "{error}");
    assert!(client.judged_ids.lock().unwrap().is_empty());
}

#[test]
#[should_panic(expected = "Unknown subagent type 'acceptance-reauthor-unregistered'")]
fn an_author_key_no_registry_defines_fails_the_scripted_author() {
    resolve_in_bare_project("acceptance-reauthor-unregistered");
}

#[test]
fn the_author_key_resolves_to_a_read_only_host_agent_in_a_bare_project() {
    let def = resolve_in_bare_project(archon_core::agents::harness::ACCEPTANCE_REAUTHOR_AGENT);
    assert_eq!(
        def.allowed_tools.as_deref(),
        Some(&["Read".to_string(), "Grep".into(), "Glob".into()][..]),
        "the resolved agent reads and searches only: no Write, Edit or Bash"
    );
}
