use super::*;
use archon_workflow::task_set_contract::AcceptanceCriterion;

struct RefutingJudge {
    batches: Mutex<Vec<Vec<String>>>,
}

fn expected_logic_identity() -> serde_json::Value {
    serde_json::json!(
        crate::command::workflow_task_set::workflow_acceptance_check_reuse::logic_identity()
            .unwrap()
    )
}

fn assert_recorded_current_logic_identity(entry: &AcceptanceCriterion) {
    assert_eq!(
        entry
            .judgment
            .sampling
            .as_ref()
            .and_then(|sampling| sampling.get("logic_identity")),
        Some(&expected_logic_identity())
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
    assert_recorded_current_logic_identity(&judged.acceptance[0]);
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
    assert_recorded_current_logic_identity(&judged.acceptance[0]);
}
