use super::*;

#[tokio::test]
async fn passability_audit_reuses_the_same_evidence_and_rejudges_changed_output() {
    let (project, repository, tasks, prd) = outside_set("echo feature-missing >&2; exit 1");
    let contract: AcceptanceContract =
        serde_json::from_slice(&std::fs::read(tasks.join(ACCEPTANCE_CONTRACT_FILE)).unwrap())
            .unwrap();
    let client = EvidenceJudge::default();
    let audit = tempfile::tempdir().unwrap();
    let runs = |stderr: &[u8]| {
        let result = archon_workflow::acceptance_scratch::CheckResult {
            acceptance_id: "AC-X-001".into(),
            exit_code: Some(1),
            quota_walk_count: 0,
            stdout: Vec::new(),
            stderr: stderr.to_vec(),
            environment_note: None,
            operational_error: None,
            fault: None,
            classification: None,
        };
        let input_key = archon_workflow::task_set_contract::content_digest(
            &serde_json::to_vec(&result).unwrap(),
        );
        crate::command::workflow_task_set::executability::BaselineRuns {
            reuse: BTreeMap::from([(
                "AC-X-001".into(),
                crate::command::workflow_task_set::executability::ReuseAudit {
                    reused: true,
                    why: "test evidence".into(),
                    evidence: None,
                    input_key: Some(input_key),
                },
            )]),
            commit: "tree-a".into(),
            failures: BTreeMap::from([(
                "AC-X-001".into(),
                crate::command::workflow_task_set::executability::CheckEvidence {
                    result,
                    copy_layout: None,
                },
            )]),
            environment: BTreeMap::new(),
            forwarded: Vec::new(),
        }
    };
    let ids = BTreeSet::from(["AC-X-001".to_string()]);
    let first = runs(b"feature missing");
    for _ in 0..2 {
        crate::command::workflow_task_set::passability::cannot_pass_findings(
            &client,
            "model-x",
            repository.path(),
            project.path(),
            &contract,
            &ids,
            &first,
            &std::fs::read_to_string(&prd).unwrap(),
            Some(audit.path()),
        )
        .await
        .unwrap();
    }
    assert_eq!(client.evidence.load(SeqCst), 1);
    let changed = runs(b"feature still missing");
    crate::command::workflow_task_set::passability::cannot_pass_findings(
        &client,
        "model-x",
        repository.path(),
        project.path(),
        &contract,
        &ids,
        &changed,
        &std::fs::read_to_string(&prd).unwrap(),
        Some(audit.path()),
    )
    .await
    .unwrap();
    assert_eq!(client.evidence.load(SeqCst), 2);
}
