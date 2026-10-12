use super::*;
use archon_workflow::task_set_contract::JudgeDecision;
use std::collections::BTreeSet;
use std::sync::Mutex;

fn cache_contract() -> AcceptanceContract {
    serde_json::from_value(serde_json::json!({
        "schema_version": 1,
        "prd": {"path": "p", "digest": "d"},
        "gap_policy": {},
        "acceptance": [{
            "id": "AC-1", "criterion": "acceptance", "check": {"kind": "command", "command": "true", "cwd": "project_root"},
            "judgment": {"verdict": "accepted", "counterexample": "", "reason": "", "host_call_id": "old"}
        }],
        "supplementary": [{
            "id": "SUP-1", "criterion": "supplementary", "check": {"kind": "command", "command": "true", "cwd": "project_root"},
            "judgment": {"verdict": "refuted", "counterexample": "", "reason": "", "host_call_id": "old"}
        }]
    }))
    .unwrap()
}
struct RecordingJudge {
    batches: Mutex<Vec<Vec<String>>>,
    omit_supplementary: bool,
}
#[async_trait]
impl WorkflowLlmClient for RecordingJudge {
    /// Scripted replies stand for one continued session (#241).
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
        // The evidence pass (Issue 275) is not the batch reuse is about.
        if text.contains("[begin untrusted program output]") {
            return Ok(
                crate::command::workflow_task_set::passability_tests_support::all_accepted(text),
            );
        }
        let checks: Vec<serde_json::Value> =
            serde_json::from_str(text.split("Checks: ").last().unwrap()).unwrap();
        let ids = checks
            .iter()
            .map(|c| c["id"].as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        self.batches.lock().unwrap().push(ids.clone());
        let decisions = ids
            .iter()
            .filter(|id| !self.omit_supplementary || !id.starts_with("SUP-"))
            .map(|id| {
                serde_json::json!({
                    "id": id,
                    "verdict": "accepted",
                    "counterexample": "no passing false state",
                    "reason": "verified"
                })
            })
            .collect::<Vec<_>>();
        Ok(WorkflowAgentOutcome {
            content: serde_json::json!({"decisions": decisions}).to_string(),
            stop_reason: Some("end_turn".into()),
            ..Default::default()
        })
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
    std::fs::write(&prd,"## Requirements\n| ID | Requirement |\n|---|---|\n| REQ-X-001 | output is valid |\n\n## Acceptance Criteria\n| ID | Criterion |\n|---|---|\n| AC-X-001 | output is valid |\n| AC-X-002 | second output is valid |\n").unwrap();
    let mut contract: AcceptanceContract = serde_json::from_slice(&original).unwrap();
    contract.acceptance[0].covers = vec!["REQ-X-001".into()];
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
        omit_supplementary: false,
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
    // The prepared round is not published. A later preparation still has
    // the saved entry verdict and does not ask the judge again.
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
    assert_eq!(judge.batches.lock().unwrap().len(), 1);
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
async fn judge_reuse_refuses_saved_verdict_with_mismatched_key_and_rejudges() {
    let (temp, tasks, prd, mut contract, judge) = frozen().await;
    contract.acceptance[1].check = archon_workflow::task_set_contract::AcceptanceCheck::Command {
        command: "jq -e '.valid == true and .count > 0' out.json".into(),
        cwd: archon_workflow::task_set_contract::TrustedCwd::ProjectRoot,
    };
    let key = super::super::incremental::entry_key(
        &*judge,
        &tasks.canonicalize().unwrap(),
        &contract.prd.digest,
        &contract.acceptance[1],
    )
    .unwrap();
    let cache = temp
        .path()
        .join(crate::command::workflow_freeze_budget::FREEZE_CACHE_DIR)
        .join("judge-entry");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(
        cache.join(format!("{key}.json")),
        serde_json::json!({
            "key": "different-computed-key",
            "judgment": contract.acceptance[1].judgment,
        })
        .to_string(),
    )
    .unwrap();

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

    assert_eq!(
        *judge.batches.lock().unwrap(),
        vec![vec!["AC-X-002".to_string()]],
        "a saved verdict bound to a different key must be judged again"
    );
}
#[tokio::test]
async fn scoped_verdict_reuse_does_not_require_a_published_pin() {
    let (temp, tasks, prd, contract, judge) = frozen().await;
    std::fs::remove_file(tasks.join(ACCEPTANCE_CONTRACT_FILE)).unwrap();
    std::fs::remove_file(acceptance_pin_path(temp.path(), &tasks)).unwrap();
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
    assert!(judge.batches.lock().unwrap().is_empty());
}

#[test]
fn cache_key_binds_canonical_task_root_and_prd_digest() {
    let temp = tempfile::tempdir().unwrap();
    let tasks_a = temp.path().join("tasks/a");
    let tasks_b = temp.path().join("tasks/b");
    std::fs::create_dir_all(&tasks_a).unwrap();
    std::fs::create_dir_all(&tasks_b).unwrap();
    let client = RecordingJudge {
        batches: Mutex::new(vec![]),
        omit_supplementary: false,
    };
    let entry = cache_contract().acceptance[0].clone();
    let a = super::super::incremental::entry_key(
        &client,
        &tasks_a.canonicalize().unwrap(),
        "prd-digest",
        &entry,
    );
    let b = super::super::incremental::entry_key(
        &client,
        &tasks_b.canonicalize().unwrap(),
        "prd-digest",
        &entry,
    );
    let changed_prd = super::super::incremental::entry_key(
        &client,
        &tasks_a.canonicalize().unwrap(),
        "other-prd",
        &entry,
    );
    assert_ne!(a, b);
    assert_ne!(a, changed_prd);
}

#[test]
fn pinned_verdict_restoration_is_never_written_as_this_rounds_verdict() {
    let temp = tempfile::tempdir().unwrap();
    let tasks = temp.path().join("tasks/set");
    std::fs::create_dir_all(&tasks).unwrap();
    let tasks = tasks.canonicalize().unwrap();
    let client = RecordingJudge {
        batches: Mutex::new(vec![]),
        omit_supplementary: false,
    };
    let mut current = cache_contract();
    let mut pinned = current.clone();
    for entry in &mut pinned.acceptance {
        entry.judgment.sampling = Some(serde_json::json!({
            "model": client.resolve_model_alias("sonnet"),
            "provider": client.provider_id()
        }));
    }
    current.acceptance[0].judgment.verdict = JudgeDecision::Refuted;
    let restored = super::super::incremental::keep_pinned_entries(&mut current, &pinned, &client);
    assert!(restored.contains("AC-1"));
    let judged = BTreeSet::from(["AC-1".to_string()]);
    super::super::incremental::save_judged_entries(
        temp.path(),
        &tasks,
        &current.prd.digest,
        &client,
        &current,
        &judged,
        &restored,
    )
    .unwrap();
    let key = super::super::incremental::entry_key(
        &client,
        &tasks,
        &current.prd.digest,
        &current.acceptance[0],
    )
    .unwrap();
    assert!(super::super::incremental::load_entry(temp.path(), &key).is_none());
}

#[test]
fn pin_reuse_covers_supplementary_checks_too() {
    let client = RecordingJudge {
        batches: Mutex::new(vec![]),
        omit_supplementary: false,
    };
    let mut current = cache_contract();
    let mut pinned = current.clone();
    let supplementary = &mut pinned.supplementary[0];
    supplementary.judgment.verdict = JudgeDecision::Accepted;
    supplementary.judgment.sampling = Some(serde_json::json!({
        "model": client.resolve_model_alias("sonnet"),
        "provider": client.provider_id()
    }));
    current.supplementary[0].judgment.verdict = JudgeDecision::Refuted;
    let restored = super::super::incremental::keep_pinned_entries(&mut current, &pinned, &client);
    assert!(restored.contains("SUP-1"));
    assert_eq!(
        current.supplementary[0].judgment.verdict,
        JudgeDecision::Accepted
    );
}

fn supplementary_only_candidate(contract: &mut AcceptanceContract) {
    let mut supplementary = contract.acceptance[0].clone();
    supplementary.id = "SUP-REQ-X-001".into();
    supplementary.criterion = "The output satisfies REQ-X-001.".into();
    supplementary.covers = vec!["REQ-X-001".into()];
    // #417 reuses a saved verdict by what the judge reads (criterion, check,
    // covers), not by id. The preflight gives this check REQ-X-001's text,
    // which equals AC-X-001's, so it needs its own command to be a new check.
    if let archon_workflow::task_set_contract::AcceptanceCheck::Command { command, .. } =
        &mut supplementary.check
    {
        command.push_str(" && test -s out.json");
    }
    supplementary.judgment.verdict = JudgeDecision::Refuted;
    supplementary.judgment.counterexample.clear();
    supplementary.judgment.reason.clear();
    supplementary.judgment.host_call_id.clear();
    supplementary.judgment.sampling = None;
    contract.supplementary.push(supplementary);
}

#[tokio::test]
async fn supplementary_only_subset_is_judged_and_merged_with_reused_acceptance() {
    let (temp, tasks, prd, mut contract, judge) = frozen().await;
    let reused = contract.acceptance[0].judgment.clone();
    supplementary_only_candidate(&mut contract);

    let prepared = prepare_acceptance_freeze_from_candidate(
        temp.path(),
        &tasks,
        &prd,
        GateMode::Observe,
        serde_json::to_vec(&contract).unwrap(),
        judge.clone(),
    )
    .await
    .expect("the merged contract passes full acceptance validation");

    assert_eq!(
        *judge.batches.lock().unwrap(),
        vec![vec!["SUP-REQ-X-001".to_string()]],
        "the accepted AC verdict is reused and only the new supplementary check is judged"
    );
    let merged: AcceptanceContract = serde_json::from_slice(&prepared.contract_bytes).unwrap();
    assert_eq!(merged.acceptance[0].judgment, reused);
    assert_eq!(
        merged.supplementary[0].judgment.verdict,
        JudgeDecision::Accepted
    );
}

#[tokio::test]
async fn supplementary_only_reply_missing_its_judgment_is_refused() {
    let (temp, tasks, prd, mut contract, _) = frozen().await;
    supplementary_only_candidate(&mut contract);
    let judge = Arc::new(RecordingJudge {
        batches: Mutex::new(vec![]),
        omit_supplementary: true,
    });

    let error = prepare_acceptance_freeze_from_candidate(
        temp.path(),
        &tasks,
        &prd,
        GateMode::Observe,
        serde_json::to_vec(&contract).unwrap(),
        judge.clone(),
    )
    .await
    .expect_err("a reply omitting the supplementary judgment is refused");

    assert!(
        format!("{error:#}").contains("missing=[\"SUP-REQ-X-001\"]"),
        "{error:#}"
    );
    // #385: the same rejection class on every reply is no progress, so the
    // judge is asked once and then re-asked for the whole no-progress window
    // before the batch ends incomplete; each ask names only the new check.
    assert_eq!(
        judge.batches.lock().unwrap().as_slice(),
        // 1 + JUDGE_NO_PROGRESS_WINDOW (3) in workflow_task_set_judge.rs.
        vec![vec!["SUP-REQ-X-001".to_string()]; 4]
    );
}
