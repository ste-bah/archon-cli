//! These tests go through freeze preparation, not just its policy helper.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Default)]
struct NoopExecutabilityProbe;

#[async_trait]
impl crate::command::workflow_task_set::executability::ExecutabilityProbe
    for NoopExecutabilityProbe
{
    async fn script_defects(
        &self,
        _contract: &AcceptanceContract,
        _ids: &std::collections::BTreeSet<String>,
    ) -> std::collections::BTreeMap<String, String> {
        std::collections::BTreeMap::new()
    }
}

async fn reject_check(check: serde_json::Value, expected: &[&str]) {
    let temp = tempfile::tempdir().unwrap();
    let (tasks, prd, bytes) = seed(&temp);
    let mut candidate: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    candidate["acceptance"][0]["check"] = check;
    let calls = Arc::new(AtomicUsize::new(0));
    let error = prepare_acceptance_freeze_from_candidate(
        temp.path(), &tasks, &prd, GateMode::Observe,
        serde_json::to_vec(&candidate).unwrap(),
        Arc::new(FinishReasonJudge {
            calls: calls.clone(), stop_reason: Some("end_turn".into()),
            content: r#"{"decisions":[{"id":"AC-X-001","verdict":"accepted","counterexample":"none","reason":"ok"}]}"#.into(),
        }),
    ).await.expect_err("mechanical defect must be refused before judging");
    assert!(CandidateRejected::caused(&error), "{error:#}");
    for text in expected {
        assert!(format!("{error:#}").contains(text), "{error:#}");
    }
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "judge must never see this candidate"
    );
    assert!(!tasks.join(ACCEPTANCE_LOCK_FILE).exists());
    assert_eq!(
        std::fs::read(tasks.join(ACCEPTANCE_CONTRACT_FILE)).unwrap(),
        bytes
    );
}

#[tokio::test]
async fn presence_floors_are_refused_before_the_judge_even_with_positive_instances() {
    for extra in [
        serde_json::json!({"min_instances":1}),
        serde_json::json!({
            "instance_source_path":"instances.json", "instance_source_records_field":"items",
            "instance_artifact_field":"path", "min_instances":1
        }),
    ] {
        let mut floor = serde_json::json!({"kind":"artifact", "artifact_path":"out.json", "required_true_fields":["ready"]});
        floor
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        reject_check(
            serde_json::json!({"kind":"floor", "contract":floor}),
            &["AC-X-001", "typed_verifier_command"],
        )
        .await;
    }
}

#[tokio::test]
async fn invented_check_fields_are_named_before_any_judge_call() {
    for key in ["command", "command_semantics", "produced_by"] {
        let mut floor = serde_json::json!({"kind":"artifact", "artifact_path":"out.json", "typed_verifier_command":"./verify-output"});
        floor[key] = "./actually-test-output".into();
        reject_check(
            serde_json::json!({"kind":"floor", "contract":floor}),
            &["unknown field", key],
        )
        .await;
    }
    reject_check(serde_json::json!({"kind":"command", "command":"./verify-output", "cwd":"project_root", "produced_by":"./build-output"}), &["unknown field", "produced_by"]).await;
}

#[tokio::test]
async fn missing_ids_and_example_ids_never_reach_the_judge() {
    for empty in [true, false] {
        let temp = tempfile::tempdir().unwrap();
        let (tasks, prd, bytes) = seed(&temp);
        let mut candidate: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        if empty {
            candidate["acceptance"] = serde_json::json!([]);
        } else {
            candidate["acceptance"][0]["id"] = "<exact acceptance id defined by the PRD>".into();
        }
        let calls = Arc::new(AtomicUsize::new(0));
        let error = prepare_acceptance_freeze_from_candidate(
            temp.path(),
            &tasks,
            &prd,
            GateMode::Observe,
            serde_json::to_vec(&candidate).unwrap(),
            Arc::new(FinishReasonJudge {
                calls: calls.clone(),
                content: String::new(),
                stop_reason: Some("end_turn".into()),
            }),
        )
        .await
        .unwrap_err();
        assert!(CandidateRejected::caused(&error), "{error:#}");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn changed_check_refutation_is_retained_for_the_acceptance_author() {
    let temp = tempfile::tempdir().unwrap();
    let (tasks, prd, original) = seed(&temp);
    freeze_acceptance(temp.path(), &tasks, &prd, Arc::new(JudgeClient {
        result: Ok(r#"{"decisions":[{"id":"AC-X-001","verdict":"accepted","counterexample":"none","reason":"checks the output"}]}"#.into()),
    })).await.unwrap();
    let first: AcceptanceContract =
        serde_json::from_slice(&std::fs::read(tasks.join(ACCEPTANCE_CONTRACT_FILE)).unwrap())
            .unwrap();
    let mut candidate: serde_json::Value = serde_json::from_slice(&original).unwrap();
    candidate["acceptance"][0]["check"]["command"] = "./weaker-verifier".into();
    let next = prepare_acceptance_freeze_from_candidate(temp.path(), &tasks, &prd, GateMode::Observe,
        serde_json::to_vec(&candidate).unwrap(), Arc::new(JudgeClient {
            result: Ok(r#"{"decisions":[{"id":"AC-X-001","verdict":"refuted","counterexample":"stale output passes","reason":"does not exercise the program"}]}"#.into()),
        })).await.unwrap();
    let restored: AcceptanceContract = serde_json::from_slice(&next.contract_bytes).unwrap();
    println!(
        "EVIDENCE #419 changed-check BEFORE: published check={:?}; verdict={:?}; candidate check={:?}",
        first.acceptance[0].check,
        first.acceptance[0].judgment.verdict,
        candidate["acceptance"][0]["check"]
    );
    assert_ne!(restored.acceptance[0].check, first.acceptance[0].check);
    assert_eq!(
        restored.acceptance[0].judgment.verdict,
        archon_workflow::task_set_contract::JudgeDecision::Refuted
    );
    assert_eq!(
        restored.acceptance[0].judgment.counterexample,
        "stale output passes"
    );
    assert!(next.non_accepted_ids().contains("AC-X-001"));
    assert!(
        next.findings.iter().any(|finding| {
            finding.subject == "AC-X-001"
                && finding.text.contains("stale output passes")
                && finding.text.contains("does not exercise the program")
        }),
        "the fresh refutation must reach the acceptance author: {:?}",
        next.findings
    );

    let scope =
        crate::command::workflow_task_set::reauthor::AuthorScope::for_task_set_without_cache(
            temp.path(),
            &tasks,
            &prd,
        )
        .unwrap();
    let author = Arc::new(
        crate::command::workflow_task_set::reauthor::test_client::ScriptedAuthorJudge::new(
            |entry, _| {
                crate::command::workflow_task_set::reauthor::test_client::command_entry(
                    entry,
                    "jq -e '.valid == true and .rows > 0' out.json",
                )
            },
            |_, _| true,
        ),
    );
    let probe = NoopExecutabilityProbe;
    let seeds = std::collections::BTreeMap::new();
    let repaired = crate::command::workflow_task_set::reauthor::reauthor(
        author.as_ref(),
        &restored,
        &std::collections::BTreeSet::from(["AC-X-001".to_string()]),
        &scope,
        "sonnet",
        &crate::command::workflow_task_set::reauthor::ReauthorGate {
            probe: &probe,
            seeds: &seeds,
        },
    )
    .await
    .unwrap();
    let prompt = author.prompts.lock().unwrap()[0].clone();
    assert!(
        prompt.contains("stale output passes"),
        "author prompt omitted the refutation: {prompt}"
    );
    assert!(
        prompt.contains("does not exercise the program"),
        "author prompt omitted the reason: {prompt}"
    );
    assert_eq!(author.authored(), 1);
    assert_eq!(
        repaired.acceptance[0].judgment.verdict,
        archon_workflow::task_set_contract::JudgeDecision::Accepted
    );
    println!(
        "EVIDENCE #419 changed-check AFTER: fresh verdict={:?}; routed_to_author={}; feedback_has_counterexample={}; feedback_has_reason={}; repaired verdict={:?}",
        restored.acceptance[0].judgment.verdict,
        author.authored() == 1,
        prompt.contains("stale output passes"),
        prompt.contains("does not exercise the program"),
        repaired.acceptance[0].judgment.verdict
    );

    let unchanged = prepare_acceptance_freeze_from_candidate(
        temp.path(),
        &tasks,
        &prd,
        GateMode::Observe,
        original,
        Arc::new(JudgeClient {
            result: Ok(r#"{"decisions":[{"id":"AC-X-001","verdict":"refuted","counterexample":"stale output passes","reason":"does not exercise the program"}]}"#.into()),
        }),
    ).await.unwrap();
    let unchanged: AcceptanceContract = serde_json::from_slice(&unchanged.contract_bytes).unwrap();
    println!(
        "EVIDENCE #419 unchanged-check BEFORE/AFTER: pinned verdict={:?}; prepared verdict={:?}; entry_equal={}",
        first.acceptance[0].judgment.verdict,
        unchanged.acceptance[0].judgment.verdict,
        unchanged.acceptance[0] == first.acceptance[0]
    );
    assert_eq!(
        unchanged.acceptance[0], first.acceptance[0],
        "unchanged check and covers restore the accepted judgment"
    );
}
