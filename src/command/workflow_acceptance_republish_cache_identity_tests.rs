use super::*;

#[test]
fn storing_a_reused_candidate_keeps_its_old_logic_stamp() {
    let set = frozen_set(&[("AC-X-001", "false", false)]);
    let scope =
        AuthorScope::for_task_set_without_cache(set.project.path(), &set.tasks, &set.prd).unwrap();
    let contract = set.contract();
    let mut saved = candidate(&contract.acceptance[0]);
    let old_stamp = serde_json::json!([9, "prior-logic", "model-x", "provider-x"]);
    saved.judgment.sampling = Some(serde_json::json!({
        "model": super::super::test_fixture::FIXTURE_JUDGE_MODEL,
        "provider": crate::command::workflow_task_set::reauthor::test_client::SCRIPTED_PROVIDER,
        "logic_identity": old_stamp,
    }));
    let probe = DataProbe::default();

    save_with_probe(&set, &contract, &scope, &saved, &probe);

    let loaded = crate::command::workflow_task_set::reauthor::cache::load(
        &set.tasks,
        &contract.acceptance[0],
        &scope,
        super::super::test_fixture::FIXTURE_JUDGE_MODEL,
        Some(crate::command::workflow_task_set::reauthor::test_client::SCRIPTED_PROVIDER),
        &probe,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        loaded.judgment.sampling.as_ref().unwrap()["logic_identity"],
        old_stamp,
        "storing a reused candidate must not make its old judgment look current"
    );
}

#[test]
fn loading_a_saved_candidate_preserves_its_stored_logic_stamp() {
    let set = frozen_set(&[("AC-X-001", "false", false)]);
    let scope =
        AuthorScope::for_task_set_without_cache(set.project.path(), &set.tasks, &set.prd).unwrap();
    let contract = set.contract();
    let frozen = &contract.acceptance[0];
    let saved = candidate(frozen);
    let probe = DataProbe::default();
    save_with_probe(&set, &contract, &scope, &saved, &probe);
    let path = std::fs::read_dir(set.tasks.join("acceptance-reauthor-candidates"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let mut record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let stored_stamp = serde_json::json!({"version": 9, "digest": "prior-logic"});
    record["candidate"]["judgment"]["sampling"] = serde_json::json!({
        "model": super::super::test_fixture::FIXTURE_JUDGE_MODEL,
        "provider": crate::command::workflow_task_set::reauthor::test_client::SCRIPTED_PROVIDER,
        "logic_identity": stored_stamp,
    });
    std::fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();

    let loaded = crate::command::workflow_task_set::reauthor::cache::load(
        &set.tasks,
        frozen,
        &scope,
        super::super::test_fixture::FIXTURE_JUDGE_MODEL,
        Some(crate::command::workflow_task_set::reauthor::test_client::SCRIPTED_PROVIDER),
        &probe,
    )
    .unwrap()
    .unwrap();

    assert_eq!(
        loaded.judgment.sampling.as_ref().unwrap()["logic_identity"],
        stored_stamp,
        "a cache hit must retain the identity under which it was judged"
    );
}

#[tokio::test]
async fn fresh_cache_miss_reauthor_carries_the_judgment_identity_stamp() {
    let set = frozen_set(&[("AC-X-001", "false", false)]);
    let ids = ids(&["AC-X-001"]);
    let scope =
        AuthorScope::for_task_set_without_cache(set.project.path(), &set.tasks, &set.prd).unwrap();
    let contract = set.contract();
    crate::command::workflow_task_set::reauthor::cache::remove(&set.tasks).unwrap();
    let client = ScriptedAuthorJudge::new(
        |entry, _| command_entry(entry, "jq -e '.ok == true' output.json"),
        |_, _| true,
    );

    let judged = crate::command::workflow_task_set::reauthor::reauthor_with_cache(
        &client,
        &contract,
        &ids,
        &scope,
        super::super::test_fixture::FIXTURE_JUDGE_MODEL,
        &set.gate(),
        Some(&set.tasks),
    )
    .await
    .unwrap();

    let sampling = judged.acceptance[0].judgment.sampling.as_ref().unwrap();
    assert_eq!(
        sampling.get("logic_identity"),
        Some(
            &crate::command::workflow_task_set::reauthor::cache::acceptance_logic_identity(
                crate::command::workflow_task_set::workflow_acceptance_check_reuse::logic_identity(
                )
                .unwrap(),
                super::super::test_fixture::FIXTURE_JUDGE_MODEL,
                Some(crate::command::workflow_task_set::reauthor::test_client::SCRIPTED_PROVIDER),
            )
        )
    );
    assert!(sampling.get("named_file_digest").is_some());
}

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
            &crate::command::workflow_task_set::reauthor::cache::acceptance_logic_identity(
                crate::command::workflow_task_set::workflow_acceptance_check_reuse::logic_identity(
                )
                .unwrap(),
                super::super::test_fixture::FIXTURE_JUDGE_MODEL,
                Some(crate::command::workflow_task_set::reauthor::test_client::SCRIPTED_PROVIDER),
            )
        ),
        "a reused candidate keeps the judge instruction identity in its judgment"
    );
}
