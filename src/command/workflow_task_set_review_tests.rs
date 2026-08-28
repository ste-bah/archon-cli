use super::*;

#[tokio::test]
async fn publication_permit_cannot_authorize_a_different_prepared_freeze() {
    let first = tempfile::tempdir().unwrap();
    let (first_tasks, first_prd, _) = seed(&first);
    let second = tempfile::tempdir().unwrap();
    let (second_tasks, second_prd, _) = seed(&second);
    let response = r#"{"decisions":[{"id":"AC-X-001","verdict":"refuted","counterexample":"false state","reason":"weak"}]}"#;

    let first_prepared = prepare_acceptance_freeze(
        first.path(),
        &first_tasks,
        &first_prd,
        archon_core::config::GateMode::Observe,
        Arc::new(JudgeClient {
            result: Ok(response.into()),
        }),
    )
    .await
    .unwrap();
    let second_prepared = prepare_acceptance_freeze(
        second.path(),
        &second_tasks,
        &second_prd,
        archon_core::config::GateMode::Observe,
        Arc::new(JudgeClient {
            result: Ok(response.into()),
        }),
    )
    .await
    .unwrap();
    assert_eq!(
        first_prepared
            .findings
            .iter()
            .map(|finding| &finding.text)
            .collect::<Vec<_>>(),
        second_prepared
            .findings
            .iter()
            .map(|finding| &finding.text)
            .collect::<Vec<_>>()
    );

    let findings = first_prepared.findings.clone();
    let identity = first_prepared.publication_identity();
    let mut disposition = crate::command::workflow_gate::run_sync_gate(
        first.path(),
        archon_core::config::GateMode::Observe,
        crate::command::workflow_gate::GateId::FreezeAcceptance,
        || {
            Ok(
                crate::command::workflow_gate::GateEvaluation::new("", findings)
                    .with_publication_identity(identity),
            )
        },
    )
    .unwrap();
    let permit = disposition.take_publication_permit().unwrap();

    let error = publish_acceptance_freeze(second_prepared, permit)
        .unwrap_err()
        .to_string();
    assert!(error.contains("does not authorize"), "{error}");
    assert!(!second_tasks.join(ACCEPTANCE_LOCK_FILE).exists());
}

#[tokio::test]
async fn malformed_prd_obligation_is_rejected_by_shared_phase_zero() {
    let temp = tempfile::tempdir().unwrap();
    let (tasks, prd, _) = seed(&temp);
    let mut body = std::fs::read_to_string(&prd).unwrap();
    body.push_str("\n## Requirements\n- REQ-X2-002: malformed area.\n");
    std::fs::write(&prd, body).unwrap();

    let error = prepare_acceptance_freeze(
        temp.path(),
        &tasks,
        &prd,
        archon_core::config::GateMode::Observe,
        Arc::new(JudgeClient {
            result: Ok(r#"{"decisions":[{"id":"AC-X-001","verdict":"accepted","counterexample":"attempted","reason":"rejects"}]}"#.into()),
        }),
    )
    .await
    .unwrap_err()
    .to_string();

    assert!(error.contains("REQ-X2-002"), "{error}");
    assert!(error.contains("REQ-<LETTERS>-<NNN>"), "{error}");
    assert!(error.contains("rename"), "{error}");
}

#[tokio::test]
async fn duplicate_prd_acceptance_id_is_rejected_by_shared_phase_zero() {
    let temp = tempfile::tempdir().unwrap();
    let (tasks, prd, _) = seed(&temp);
    let mut body = std::fs::read_to_string(&prd).unwrap();
    body.push_str("| AC-X-001 | conflicting duplicate criterion |\n");
    std::fs::write(&prd, body).unwrap();

    let error = prepare_acceptance_freeze(
        temp.path(),
        &tasks,
        &prd,
        archon_core::config::GateMode::Observe,
        Arc::new(JudgeClient {
            result: Ok(r#"{"decisions":[{"id":"AC-X-001","verdict":"accepted","counterexample":"attempted","reason":"rejects"}]}"#.into()),
        }),
    )
    .await
    .unwrap_err()
    .to_string();

    assert!(error.contains("AC-X-001"), "{error}");
    assert!(
        error.contains("more than one obligation-table row"),
        "{error}"
    );
    assert!(error.contains("keep exactly one row"), "{error}");
}
