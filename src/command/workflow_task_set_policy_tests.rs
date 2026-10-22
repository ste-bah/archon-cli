use super::*;

#[tokio::test]
async fn observe_freeze_stamps_policy_findings_and_enforce_requires_refreeze() {
    use archon_workflow::task_set_contract::{AcceptanceLock, FreezeGateMode, TASK_SKELETON_FILE};

    let temp = tempfile::tempdir().unwrap();
    let (tasks, prd, _) = seed(&temp);
    std::fs::write(&prd, "## Acceptance Criteria\n| ID | Criterion |\n|---|---|\n| AC-X-001 | A commandless floor with required_true_fields. |\n").unwrap();
    let draft = std::fs::read_to_string(tasks.join(ACCEPTANCE_CONTRACT_FILE)).unwrap();
    let weak = draft.replace("jq -e '.valid == true' out.json", "true");
    std::fs::write(tasks.join(ACCEPTANCE_CONTRACT_FILE), weak).unwrap();
    let prepared = prepare_acceptance_freeze(
        temp.path(),
        &tasks,
        &prd,
        archon_core::config::GateMode::Observe,
        Arc::new(JudgeClient {
            result: Ok(r#"{"decisions":[{"id":"AC-X-001","verdict":"accepted","counterexample":"attempted","reason":"accepted for shadow measurement"}]}"#.into()),
        }),
    )
    .await
    .unwrap();
    // Three findings, not one: the floor defect, the contradiction of a
    // judge returning `accepted` for the very criterion the policy layer
    // reports (a verdict cannot outrank a defect the host checked itself),
    // and the host's own: the probe runs no check carrying a policy defect,
    // so it is unproven on the pre-implementation tree.
    assert_eq!(prepared.findings.len(), 3, "{:?}", prepared.findings);
    assert!(
        (prepared.findings.iter()).any(|finding| finding.remediation_scope
            == archon_workflow::RemediationScope::Operational
            && finding.text.contains(super::executability::HOST_UNPROVEN)),
        "{:?}",
        prepared.findings
    );
    assert!(
        prepared.findings.iter().any(|finding| finding
            .text
            .contains("contradicts a finding the host verified")),
        "the judge/policy disagreement must itself be a finding: {:?}",
        prepared.findings
    );
    let findings = prepared.findings.clone();
    let publication_identity = prepared.publication_identity();
    let mut disposition = crate::command::workflow_gate::run_sync_gate(
        temp.path(),
        archon_core::config::GateMode::Observe,
        crate::command::workflow_gate::GateId::FreezeAcceptance,
        || {
            Ok(
                crate::command::workflow_gate::GateEvaluation::new("", findings)
                    .with_publication_identity(publication_identity),
            )
        },
    )
    .unwrap();
    let permit = disposition.take_publication_permit().unwrap();
    publish_acceptance_freeze(prepared, permit).unwrap();
    let lock: AcceptanceLock =
        serde_json::from_slice(&std::fs::read(tasks.join(ACCEPTANCE_LOCK_FILE)).unwrap()).unwrap();
    assert_eq!(lock.gate.mode, FreezeGateMode::Observe);
    // Three: the floor defect, the judge contradicting it, and the host's
    // unproven check, each stamped on the lock.
    assert_eq!(lock.gate.finding_count, 3);
    assert!(!lock.gate.findings_digest.is_empty());

    std::fs::write(
        tasks.join(TASK_SKELETON_FILE),
        r#"{"schema_version":1,"acceptance_digest":"draft","tasks":[{"task_id":"TASK-X-010","file_name":"TASK-X-010-body.md","depends_on":[],"blocks":[],"implements":["AC-X-001"],"deliverable_contracts":[]}]}"#,
    )
    .unwrap();
    let prepared = prepare_skeleton_freeze(
        temp.path(),
        &tasks,
        &prd,
        archon_core::config::GateMode::Enforce,
    )
    .unwrap();
    let finding = prepared
        .findings
        .iter()
        .find(|finding| finding.text.contains("predecessor acceptance freeze"))
        .expect("predecessor finding");
    assert!(
        finding.text.contains("re-freeze under enforce"),
        "{}",
        finding.text
    );
}
