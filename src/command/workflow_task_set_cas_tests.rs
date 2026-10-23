use super::*;

#[tokio::test]
async fn judged_reprepare_keeps_the_pre_reauthor_contract_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    let (tasks, prd, original) = seed(&temp);
    std::fs::write(temp.path().join("out.json"), r#"{"valid":true}"#).unwrap();
    crate::command::workflow_task_set::passability_tests_support::commit_project(temp.path());
    let initial = prepare_acceptance_freeze_from_candidate(
        temp.path(),
        &tasks,
        &prd,
        GateMode::Observe,
        original.clone(),
        Arc::new(JudgeClient {
            result: Ok(r#"{"decisions":[{"id":"AC-X-001","verdict":"accepted","counterexample":"false is rejected","reason":"checks validity"}]}"#.into()),
        }),
    )
    .await
    .unwrap();
    let before_reauthor = initial.live_contract_digest.clone();

    let mut newer_value: serde_json::Value = serde_json::from_slice(&original).unwrap();
    newer_value["acceptance"][0]["check"]["command"] = "check --new-contract".into();
    let newer = serde_json::to_vec_pretty(&newer_value).unwrap();
    std::fs::write(tasks.join(ACCEPTANCE_CONTRACT_FILE), &newer).unwrap();
    let mut judged: archon_workflow::task_set_contract::AcceptanceContract =
        serde_json::from_slice(&original).unwrap();
    judged.acceptance[0].criterion = "output is valid".into();
    judged.acceptance[0].judgment = archon_workflow::task_set_contract::JudgeVerdict {
        verdict: archon_workflow::task_set_contract::JudgeDecision::Accepted,
        counterexample: "invalid output is rejected".into(),
        reason: "the check enforces validity".into(),
        host_call_id: "test-judge".into(),
        sampling: None,
    };
    let reauthored = prepare_from_judged(
        temp.path(),
        &tasks,
        &prd,
        GateMode::Observe,
        &judged,
        Some("baseline".into()),
        before_reauthor.clone(),
    )
    .unwrap();

    let error = publish_observed(reauthored, temp.path())
        .expect_err("a writer's contract change during reauthoring makes this publish stale");
    assert!(
        error.downcast_ref::<StaleAcceptanceContract>().is_some(),
        "{error:#}"
    );
    assert_eq!(
        std::fs::read(tasks.join(ACCEPTANCE_CONTRACT_FILE)).unwrap(),
        newer
    );
    assert!(before_reauthor.is_some());
}
