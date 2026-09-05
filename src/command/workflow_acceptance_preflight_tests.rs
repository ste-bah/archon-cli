//! These tests go through freeze preparation, not just its policy helper.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

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
async fn later_refutation_keeps_the_previously_accepted_check_at_the_freeze_call_site() {
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
    assert_eq!(restored.acceptance[0], first.acceptance[0]);
    assert!(next.findings.is_empty(), "{:?}", next.findings);
}
