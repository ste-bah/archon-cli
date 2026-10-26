use super::*;

#[tokio::test]
async fn a_matching_saved_candidate_skips_author_and_candidate_judge_calls() {
    let set = frozen_set(&[("AC-X-001", "false", false)]);
    let named = ids(&["AC-X-001"]);
    let scope =
        AuthorScope::for_task_set_without_cache(set.project.path(), &set.tasks, &set.prd).unwrap();
    let contract = set.contract();
    let saved = candidate(&contract.acceptance[0]);
    crate::command::workflow_task_set::reauthor::cache::remove(&set.tasks).unwrap();
    save(&set, &contract, &scope, &saved);
    let client = ScriptedAuthorJudge::new(|entry, _| command_entry(entry, "false"), |_, _| true);
    let repaired = crate::command::workflow_task_set::reauthor::reauthor_with_cache(
        &client,
        &contract,
        &named,
        &scope,
        super::super::test_fixture::FIXTURE_JUDGE_MODEL,
        &set.gate(),
        Some(&set.tasks),
    )
    .await
    .unwrap();
    assert_eq!(client.authored(), 0);
    assert_eq!(
        *client.judged_ids.lock().unwrap(),
        ["AC-X-001"],
        "only the passability judge runs after the probe"
    );
    assert_eq!(repaired.acceptance[0].check, saved.check);
    assert_eq!(
        repaired.acceptance[0]
            .judgment
            .sampling
            .as_ref()
            .and_then(|sampling| sampling.get("logic_identity")),
        Some(
            &crate::command::workflow_task_set::reauthor::cache::judge_instruction_identity(
                super::super::test_fixture::FIXTURE_JUDGE_MODEL,
                Some(crate::command::workflow_task_set::reauthor::test_client::SCRIPTED_PROVIDER),
            )
        ),
        "a reused candidate keeps the judge instruction identity in its judgment"
    );
}
