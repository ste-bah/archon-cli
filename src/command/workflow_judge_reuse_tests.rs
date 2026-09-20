use super::*;
use std::sync::Mutex;
struct RecordingJudge {
    batches: Mutex<Vec<Vec<String>>>,
}
#[async_trait]
impl WorkflowLlmClient for RecordingJudge {
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
        let ids = checks
            .iter()
            .map(|c| c["id"].as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        self.batches.lock().unwrap().push(ids.clone());
        Ok(WorkflowAgentOutcome {content:serde_json::json!({"decisions":ids.iter().map(|id|serde_json::json!({"id":id,"verdict":"accepted","counterexample":"no passing false state","reason":"verified"})).collect::<Vec<_>>()}).to_string(),stop_reason:Some("end_turn".into()),..Default::default()})
    }
    async fn send_message_with_temperature(
        &self,
        m: Vec<serde_json::Value>,
        s: Vec<serde_json::Value>,
        t: Vec<serde_json::Value>,
        model: &str,
        _: f64,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        self.send_message(m, s, t, model).await
    }
}
async fn frozen() -> (
    tempfile::TempDir,
    PathBuf,
    PathBuf,
    AcceptanceContract,
    Arc<RecordingJudge>,
) {
    let temp = tempfile::tempdir().unwrap();
    let (tasks, prd, original) = seed(&temp);
    std::fs::write(&prd,"## Acceptance Criteria\n| ID | Criterion |\n|---|---|\n| AC-X-001 | output is valid |\n| AC-X-002 | second output is valid |\n").unwrap();
    let mut contract: AcceptanceContract = serde_json::from_slice(&original).unwrap();
    let mut second = contract.acceptance[0].clone();
    second.id = "AC-X-002".into();
    contract.acceptance.push(second);
    std::fs::write(
        tasks.join(ACCEPTANCE_CONTRACT_FILE),
        serde_json::to_vec(&contract).unwrap(),
    )
    .unwrap();
    let judge = Arc::new(RecordingJudge {
        batches: Mutex::new(vec![]),
    });
    freeze_acceptance(temp.path(), &tasks, &prd, judge.clone())
        .await
        .unwrap();
    let contract =
        serde_json::from_slice(&std::fs::read(tasks.join(ACCEPTANCE_CONTRACT_FILE)).unwrap())
            .unwrap();
    judge.batches.lock().unwrap().clear();
    (temp, tasks, prd, contract, judge)
}
#[tokio::test]
async fn judge_reuse_skips_unchanged_and_judges_only_changed_entry() {
    let (temp, tasks, prd, mut contract, judge) = frozen().await;
    prepare_acceptance_freeze_from_candidate(
        temp.path(),
        &tasks,
        &prd,
        GateMode::Observe,
        serde_json::to_vec(&contract).unwrap(),
        judge.clone(),
    )
    .await
    .unwrap();
    assert!(
        judge.batches.lock().unwrap().is_empty(),
        "unchanged contract was rejudged"
    );
    let old = contract.acceptance[0].judgment.clone();
    contract.acceptance[1].check = archon_workflow::task_set_contract::AcceptanceCheck::Command {
        command: "jq -e '.valid == true and .count > 0' out.json".into(),
        cwd: archon_workflow::task_set_contract::TrustedCwd::ProjectRoot,
    };
    let prepared = prepare_acceptance_freeze_from_candidate(
        temp.path(),
        &tasks,
        &prd,
        GateMode::Observe,
        serde_json::to_vec(&contract).unwrap(),
        judge.clone(),
    )
    .await
    .unwrap();
    assert_eq!(
        *judge.batches.lock().unwrap(),
        vec![vec!["AC-X-002".to_string()]]
    );
    let result: AcceptanceContract = serde_json::from_slice(&prepared.contract_bytes).unwrap();
    assert_eq!(result.acceptance[0].judgment, old);
}
#[tokio::test]
async fn judge_reuse_never_skips_whole_contract_consistency() {
    let (temp, tasks, prd, mut contract, judge) = frozen().await;
    contract.acceptance[0].gap_permitted = true;
    let error = prepare_acceptance_freeze_from_candidate(
        temp.path(),
        &tasks,
        &prd,
        GateMode::Observe,
        serde_json::to_vec(&contract).unwrap(),
        judge.clone(),
    )
    .await
    .unwrap_err();
    assert!(format!("{error:#}").contains("gap_permitted disagrees"));
    assert!(judge.batches.lock().unwrap().is_empty());
}
#[tokio::test]
async fn judge_reuse_refuses_unbound_disk_verdicts() {
    let (temp, tasks, prd, contract, judge) = frozen().await;
    std::fs::write(tasks.join(ACCEPTANCE_CONTRACT_FILE), "{}").unwrap();
    prepare_acceptance_freeze_from_candidate(
        temp.path(),
        &tasks,
        &prd,
        GateMode::Observe,
        serde_json::to_vec(&contract).unwrap(),
        judge.clone(),
    )
    .await
    .unwrap();
    assert_eq!(judge.batches.lock().unwrap()[0].len(), 2);
}
