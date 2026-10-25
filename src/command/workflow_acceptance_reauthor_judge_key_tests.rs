//! A saved repair is keyed by the model that judges it: the judge's alias
//! as the client's provider configuration resolves it, so a judge
//! configured for another model never reuses it.

use super::*;
use crate::command::workflow_task_set::reauthor::test_client::SCRIPTED_PROVIDER;
use archon_workflow::task_set_contract::{AcceptanceCheck, JudgeDecision, TrustedCwd};

/// Author calls of a repair of a check whose accepted repair was saved
/// under the fixture's judge model, by a client resolving every alias to
/// `resolved` (when set).
async fn authored_with(resolved: Option<&str>) -> usize {
    let set = frozen_set(&[("AC-F-001", "false", false)]);
    let scope =
        AuthorScope::for_task_set_without_cache(set.project.path(), &set.tasks, &set.prd).unwrap();
    let contract = set.contract();
    let mut saved = contract.acceptance[0].clone();
    saved.check = AcceptanceCheck::Command {
        command: "false".into(),
        cwd: TrustedCwd::ProjectRoot,
    };
    saved.judgment.verdict = JudgeDecision::Accepted;
    let judge = super::super::test_fixture::FIXTURE_JUDGE_MODEL;
    let provider = Some(SCRIPTED_PROVIDER);
    crate::command::workflow_task_set::reauthor::cache::store(
        &set.tasks, &saved, &contract, &scope, judge, provider, &set.probe,
    )
    .unwrap();
    let mut client =
        ScriptedAuthorJudge::new(|entry, _| command_entry(entry, "test -s out"), |_, _| true);
    if let Some(model) = resolved {
        client = client.with_resolved_model(model);
    }
    crate::command::workflow_task_set::reauthor::reauthor_with_cache(
        &client,
        &contract,
        &ids(&["AC-F-001"]),
        &scope,
        judge,
        &set.gate(),
        Some(&set.tasks),
    )
    .await
    .unwrap();
    client.authored()
}

#[tokio::test]
async fn a_saved_repair_is_reused_only_by_the_model_that_judged_it() {
    assert_eq!(
        authored_with(None).await,
        0,
        "the same judging model reuses it"
    );
    assert!(
        authored_with(Some("another-configured-model")).await > 0,
        "a judge configured for another model re-authors the check"
    );
}
