use super::*;
use archon_workflow::task_set_contract::AcceptanceCriterion;

struct RefutingJudge {
    batches: Mutex<Vec<Vec<String>>>,
}

fn expected_logic_identity(client: &dyn WorkflowLlmClient) -> serde_json::Value {
    crate::command::workflow_task_set::workflow_acceptance_check_reuse::logic_identity()
        .map(|logic| {
            crate::command::workflow_task_set::reauthor::cache::acceptance_logic_identity(
                logic,
                &client.resolve_model_alias("sonnet"),
                client.provider_id().as_deref(),
            )
        })
        .unwrap()
}

fn stamp_for_test(entry: &mut AcceptanceCriterion, client: &dyn WorkflowLlmClient) {
    entry.judgment.sampling = Some(serde_json::json!({
        "model": client.resolve_model_alias("sonnet"),
        "provider": client.provider_id(),
    }));
    crate::command::workflow_task_set::reauthor::cache::stamp_judgment(
        entry,
        &client.resolve_model_alias("sonnet"),
        client.provider_id().as_deref(),
        &super::super::judge::JudgeRoots::default(),
    );
}

#[test]
fn stamping_replaces_a_non_object_sampling_value() {
    let client = RefutingJudge {
        batches: Mutex::new(Vec::new()),
    };
    let mut contract = super::cache_contract();
    let mut entry = contract.acceptance.remove(0);
    entry.judgment.sampling = Some(serde_json::json!("legacy"));

    stamp_for_test(&mut entry, &client);

    let sampling = entry.judgment.sampling.unwrap();
    assert!(sampling.is_object());
    assert_eq!(sampling["logic_identity"], expected_logic_identity(&client));
    assert!(sampling.get("named_file_digest").is_some());
}

fn assert_recorded_current_logic_identity(
    entry: &AcceptanceCriterion,
    client: &dyn WorkflowLlmClient,
) {
    assert_eq!(
        entry
            .judgment
            .sampling
            .as_ref()
            .and_then(|sampling| sampling.get("logic_identity")),
        Some(&expected_logic_identity(client))
    );
}

#[test]
fn pinned_identity_uses_the_current_logic_helper_across_builds() {
    let client = RefutingJudge {
        batches: Mutex::new(Vec::new()),
    };
    let mut current = super::cache_contract();
    let base_logic =
        crate::command::workflow_task_set::workflow_acceptance_check_reuse::logic_identity()
            .unwrap();
    let first_build = crate::command::workflow_task_set::reauthor::cache::acceptance_logic_identity(
        (base_logic.0, base_logic.1.clone(), "build-a".into()),
        "resolved-judge",
        Some("provider"),
    );
    let second_build =
        crate::command::workflow_task_set::reauthor::cache::acceptance_logic_identity(
            (base_logic.0, base_logic.1, "build-b".into()),
            "resolved-judge",
            Some("provider"),
        );
    assert_eq!(first_build, second_build);
    let mut pinned = current.clone();
    let entry = &mut pinned.acceptance[0];
    entry.judgment.verdict = JudgeDecision::Accepted;
    stamp_for_test(entry, &client);
    current.acceptance[0].judgment.verdict = JudgeDecision::Refuted;

    let restored = super::super::incremental::keep_pinned_entries(
        &mut current,
        &pinned,
        &client,
        &super::super::judge::JudgeRoots::default(),
    );

    assert!(
        restored.contains(&current.acceptance[0].id),
        "same instruction identity must restore across builds"
    );
    assert_eq!(
        current.acceptance[0].judgment.verdict,
        JudgeDecision::Accepted
    );
}

#[test]
fn changed_freeze_logic_identity_keeps_the_fresh_refutation() {
    let client = RefutingJudge {
        batches: Mutex::new(Vec::new()),
    };
    let mut current = super::cache_contract();
    let mut pinned = current.clone();
    let entry = &mut pinned.acceptance[0];
    entry.judgment.verdict = JudgeDecision::Accepted;
    stamp_for_test(entry, &client);
    entry.judgment.sampling.as_mut().unwrap()["logic_identity"][1] =
        serde_json::json!("prior-freeze-logic-digest");
    current.acceptance[0].judgment.verdict = JudgeDecision::Refuted;

    let restored = super::super::incremental::keep_pinned_entries(
        &mut current,
        &pinned,
        &client,
        &super::super::judge::JudgeRoots::default(),
    );

    assert!(!restored.contains(&current.acceptance[0].id));
    assert_eq!(
        current.acceptance[0].judgment.verdict,
        JudgeDecision::Refuted
    );
}

#[test]
fn changed_freeze_logic_version_keeps_the_fresh_refutation() {
    let client = RefutingJudge {
        batches: Mutex::new(Vec::new()),
    };
    let mut current = super::cache_contract();
    let mut pinned = current.clone();
    let old = &mut pinned.acceptance[0];
    old.judgment.verdict = JudgeDecision::Accepted;
    stamp_for_test(old, &client);
    old.judgment.sampling.as_mut().unwrap()["logic_identity"][0] = serde_json::json!(0);
    current.acceptance[0].judgment.verdict = JudgeDecision::Refuted;

    let restored = super::super::incremental::keep_pinned_entries(
        &mut current,
        &pinned,
        &client,
        &super::super::judge::JudgeRoots::default(),
    );

    assert!(!restored.contains(&current.acceptance[0].id));
    assert_eq!(
        current.acceptance[0].judgment.verdict,
        JudgeDecision::Refuted
    );
}

#[test]
fn changed_named_file_keeps_the_fresh_refutation() {
    let client = RefutingJudge {
        batches: Mutex::new(Vec::new()),
    };
    let root = tempfile::tempdir().unwrap();
    let named_file = root.path().join("input");
    std::fs::write(&named_file, "judge saw this").unwrap();
    let roots = super::super::judge::JudgeRoots {
        project: Some(root.path().to_path_buf()),
        repository: None,
        task_root: Some(root.path().to_path_buf()),
        repository_commit: None,
    };
    let mut current = super::cache_contract();
    current.acceptance[0].check = archon_workflow::task_set_contract::AcceptanceCheck::Command {
        command: "operation input".into(),
        cwd: archon_workflow::task_set_contract::TrustedCwd::ProjectRoot,
    };
    let mut pinned = current.clone();
    let old = &mut pinned.acceptance[0];
    old.judgment.verdict = JudgeDecision::Accepted;
    stamp_for_test(old, &client);
    // Bind this test's pin to the actual project file named by the check.
    crate::command::workflow_task_set::reauthor::cache::stamp_judgment(
        old,
        &client.resolve_model_alias("sonnet"),
        client.provider_id().as_deref(),
        &roots,
    );
    current.acceptance[0].judgment.verdict = JudgeDecision::Refuted;
    std::fs::write(&named_file, "changed after the judge").unwrap();

    let restored =
        super::super::incremental::keep_pinned_entries(&mut current, &pinned, &client, &roots);

    assert!(!restored.contains(&current.acceptance[0].id));
    assert_eq!(
        current.acceptance[0].judgment.verdict,
        JudgeDecision::Refuted
    );
}

#[async_trait]
impl WorkflowLlmClient for RefutingJudge {
    async fn continue_agent(
        &self,
        call: archon_workflow::WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
        self.run_agent(call).await
    }

    async fn send_message(
        &self,
        messages: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: &str,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        let text = messages[0]["content"].as_str().unwrap();
        let checks: Vec<serde_json::Value> =
            serde_json::from_str(text.split("Checks: ").last().unwrap()).unwrap();
        let ids: Vec<String> = checks
            .iter()
            .map(|check| check["id"].as_str().unwrap().to_string())
            .collect();
        self.batches.lock().unwrap().push(ids.clone());
        Ok(WorkflowAgentOutcome {
            content: serde_json::json!({"decisions": ids.iter().map(|id| serde_json::json!({
                "id": id, "verdict": "refuted", "counterexample": "counterexample",
                "reason": "refuted by current logic"
            })).collect::<Vec<_>>()})
            .to_string(),
            stop_reason: Some("end_turn".into()),
            ..Default::default()
        })
    }

    async fn send_message_with_temperature(
        &self,
        messages: Vec<serde_json::Value>,
        system: Vec<serde_json::Value>,
        tools: Vec<serde_json::Value>,
        model: &str,
        _: f64,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        self.send_message(messages, system, tools, model).await
    }
}

fn rewrite_published_contract_without_judge_identity(project: &Path, tasks: &Path) {
    let contract_path = tasks.join(ACCEPTANCE_CONTRACT_FILE);
    let mut contract: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&contract_path).unwrap()).unwrap();
    for group in ["acceptance", "supplementary"] {
        for entry in contract[group].as_array_mut().unwrap() {
            if let Some(sampling) = entry["judgment"]["sampling"].as_object_mut() {
                sampling.remove("logic_identity");
            }
        }
    }
    let bytes = serde_json::to_vec(&contract).unwrap();
    std::fs::write(&contract_path, &bytes).unwrap();
    let digest = archon_workflow::task_set_contract::content_digest(&bytes);
    let lock_path = tasks.join(ACCEPTANCE_LOCK_FILE);
    let mut lock: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&lock_path).unwrap()).unwrap();
    lock["digest"] = serde_json::json!(digest);
    std::fs::write(lock_path, serde_json::to_vec(&lock).unwrap()).unwrap();
    let pin_path = acceptance_pin_path(project, tasks);
    let mut pin: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&pin_path).unwrap()).unwrap();
    pin["acceptance_digest"] = serde_json::json!(digest);
    std::fs::write(pin_path, serde_json::to_vec(&pin).unwrap()).unwrap();
}

#[tokio::test]
async fn judge_does_not_restore_a_pinned_acceptance_without_current_logic_identity() {
    let (temp, tasks, _prd, contract, _judge) = super::frozen().await;
    rewrite_published_contract_without_judge_identity(temp.path(), &tasks);
    let old_pin_contract: AcceptanceContract =
        serde_json::from_slice(&std::fs::read(tasks.join(ACCEPTANCE_CONTRACT_FILE)).unwrap())
            .unwrap();
    assert!(
        old_pin_contract.acceptance[0]
            .judgment
            .sampling
            .as_ref()
            .and_then(|sampling| sampling.get("logic_identity"))
            .is_none()
    );
    let cache = temp
        .path()
        .join(crate::command::workflow_freeze_budget::FREEZE_CACHE_DIR);
    if cache.exists() {
        std::fs::remove_dir_all(&cache).unwrap();
    }
    let client = RefutingJudge {
        batches: Mutex::new(Vec::new()),
    };
    let expected = contract
        .acceptance
        .iter()
        .chain(&contract.supplementary)
        .map(|entry| entry.id.clone())
        .collect();
    let judged = super::super::incremental::judge(
        temp.path(),
        &tasks,
        &client,
        contract,
        &expected,
        None,
        &super::super::judge::JudgeRoots::default(),
    )
    .await
    .unwrap();
    assert_eq!(
        judged.acceptance[0].judgment.verdict,
        JudgeDecision::Refuted,
        "a pin file without logic identity must not replace the fresh refutation"
    );
}

#[tokio::test]
async fn judge_restores_a_pinned_acceptance_from_the_same_logic_identity() {
    let (temp, tasks, _prd, contract, _judge) = super::frozen().await;
    let cache = temp
        .path()
        .join(crate::command::workflow_freeze_budget::FREEZE_CACHE_DIR);
    if cache.exists() {
        std::fs::remove_dir_all(&cache).unwrap();
    }
    let client = RefutingJudge {
        batches: Mutex::new(Vec::new()),
    };
    let expected = contract
        .acceptance
        .iter()
        .chain(&contract.supplementary)
        .map(|entry| entry.id.clone())
        .collect();
    let judged = super::super::incremental::judge(
        temp.path(),
        &tasks,
        &client,
        contract,
        &expected,
        None,
        &super::super::judge::JudgeRoots::default(),
    )
    .await
    .unwrap();
    assert_eq!(
        judged.acceptance[0].judgment.verdict,
        JudgeDecision::Accepted,
        "the same-identity pin preserves the anti-flap behavior"
    );
    assert_recorded_current_logic_identity(&judged.acceptance[0], &client);
}

#[tokio::test]
async fn judge_does_not_restore_after_check_text_changes() {
    let (temp, tasks, _prd, mut contract, _judge) = super::frozen().await;
    let cache = temp
        .path()
        .join(crate::command::workflow_freeze_budget::FREEZE_CACHE_DIR);
    if cache.exists() {
        std::fs::remove_dir_all(&cache).unwrap();
    }
    contract.acceptance[0].check = archon_workflow::task_set_contract::AcceptanceCheck::Command {
        command: "false".into(),
        cwd: archon_workflow::task_set_contract::TrustedCwd::ProjectRoot,
    };
    let client = RefutingJudge {
        batches: Mutex::new(Vec::new()),
    };
    let expected = contract
        .acceptance
        .iter()
        .chain(&contract.supplementary)
        .map(|entry| entry.id.clone())
        .collect();
    let judged = super::super::incremental::judge(
        temp.path(),
        &tasks,
        &client,
        contract,
        &expected,
        None,
        &super::super::judge::JudgeRoots::default(),
    )
    .await
    .unwrap();
    assert_eq!(
        judged.acceptance[0].judgment.verdict,
        JudgeDecision::Refuted,
        "a changed check keeps the fresh refutation"
    );
    assert_recorded_current_logic_identity(&judged.acceptance[0], &client);
}
