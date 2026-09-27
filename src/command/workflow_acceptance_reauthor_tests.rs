//! The bounded re-author loop over a scripted author and judge.

use super::test_client::{ScriptedAuthorJudge, command_entry};
use super::*;
use crate::command::workflow_task_set::republish::test_fixture::frozen_set;

fn scope(
    set: &crate::command::workflow_task_set::republish::test_fixture::FrozenSet,
) -> AuthorScope {
    AuthorScope::for_task_set(set.project.path(), &set.tasks, &set.prd)
}

fn ids(list: &[&str]) -> BTreeSet<String> {
    list.iter().map(|id| id.to_string()).collect()
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
    let after = reauthor(&client, &before, &ids(&["AC-F-002"]), &scope(&set))
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
        vec!["AC-F-002".to_string()]
    );
}

#[tokio::test]
async fn a_check_the_judge_keeps_refuting_fails_after_the_bound_with_a_per_check_report() {
    let set = frozen_set(&[("AC-F-001", "jq -e '.a == true' out.json", false)]);
    let client = ScriptedAuthorJudge::new(
        |entry, attempt| command_entry(entry, &format!("jq -e '.a{attempt} == true' out.json")),
        |_, _| false,
    );
    let error = reauthor(&client, &set.contract(), &ids(&["AC-F-001"]), &scope(&set))
        .await
        .expect_err("never accepted")
        .to_string();
    assert_eq!(client.authored(), REAUTHOR_ATTEMPTS);
    assert!(error.contains("AC-F-001"), "{error}");
    assert!(error.contains("after 3 re-author attempt(s)"), "{error}");
    assert!(error.contains("the check misses a branch"), "{error}");
}

#[tokio::test]
async fn a_reply_repeating_the_refuted_check_is_never_judged_and_costs_its_attempt() {
    let set = frozen_set(&[("AC-F-001", "jq -e '.a == true' out.json", false)]);
    let client = ScriptedAuthorJudge::new(
        |entry, _| command_entry(entry, "jq -e '.a == true' out.json"),
        |_, _| true,
    );
    let error = reauthor(&client, &set.contract(), &ids(&["AC-F-001"]), &scope(&set))
        .await
        .expect_err("a repeated check is not a repair")
        .to_string();
    assert!(error.contains("repeats the check"), "{error}");
    assert!(client.judged_ids.lock().unwrap().is_empty());
}
