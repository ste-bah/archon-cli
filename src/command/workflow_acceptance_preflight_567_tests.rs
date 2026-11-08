use super::*;

#[tokio::test]
async fn stale_parse_call_id_does_not_create_a_current_parse_defect_567() {
    let temp = tempfile::tempdir().unwrap();
    let (tasks, prd, bytes) = seed(&temp);
    let mut candidate: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    candidate["acceptance"][0]["judgment"]["host_call_id"] = "host:shell-parse".into();
    candidate["acceptance"][0]["check"] = serde_json::json!({"kind":"floor", "contract": {
        "kind":"artifact", "artifact_path":"out.json", "artifact_format":"", "required_true_fields":[],
        "typed_verifier_command":"./verify-output"
    }});
    let calls = Arc::new(AtomicUsize::new(0));
    let error = prepare_acceptance_freeze_from_candidate(
        temp.path(),
        &tasks,
        &prd,
        GateMode::Observe,
        serde_json::to_vec(&candidate).unwrap(),
        Arc::new(FinishReasonJudge {
            calls: calls.clone(),
            stop_reason: Some("end_turn".into()),
            content: String::new(),
        }),
    )
    .await
    .unwrap_err();
    let defects = &error
        .downcast_ref::<crate::command::workflow_task_set_candidate::CandidateDefects>()
        .unwrap()
        .0;
    assert!(!defects.is_empty());
    assert!(
        defects
            .iter()
            .any(|defect| defect.message.contains("artifact_format")),
        "{defects:?}"
    );
    assert!(
        defects
            .iter()
            .all(|defect| defect.identity.code != "shell_parse"),
        "{defects:?}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}
