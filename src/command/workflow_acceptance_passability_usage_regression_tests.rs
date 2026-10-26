use super::*;

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial]
async fn host_refutation_precedes_a_saved_accepted_audit() {
    use std::os::unix::fs::PermissionsExt;

    let repository = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let external = tempfile::tempdir().unwrap();
    let executable = external.path().join("tool");
    std::fs::write(
        &executable,
        "#!/bin/sh\necho \"error: unexpected argument '--new-flag' found\" >&2\necho 'Usage: tool [OPTIONS]' >&2\nexit 2\n",
    )
    .unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut baseline =
        super::runs("error: unexpected argument '--new-flag' found\nUsage: tool [OPTIONS]\n");
    baseline
        .environment
        .insert("PATH".into(), external.path().display().to_string());
    let contract: AcceptanceContract = serde_json::from_value(serde_json::json!({
        "schema_version": 1,
        "prd": {"path": "fixture.md", "digest": "fixture"},
        "gap_policy": {"permitted_acceptance_ids": [], "forbidden_phrases": [], "required_fields": []},
        "acceptance": [{
            "id": "AC-X-001", "criterion": "output is valid",
            "check": {"kind": "command", "command": "tool --new-flag", "cwd": "project_root"},
            "judgment": {"verdict": "accepted", "counterexample": "", "reason": "", "host_call_id": ""}
        }],
        "supplementary": []
    })).unwrap();
    let requirements = archon_workflow::v2::acceptance_stage::coverage::prd_requirement_texts("");
    let redactor = super::super::super::evidence::Redactor::for_runs(&baseline);
    let client = crate::command::workflow_task_set::passability_tests::EvidenceJudge::default();
    let candidate = super::super::super::candidates(
        &client,
        "reviewer",
        &contract,
        &BTreeSet::from(["AC-X-001".into()]),
        &baseline,
        &requirements,
        &redactor,
    )
    .into_iter()
    .next()
    .unwrap();
    let audit = tempfile::tempdir().unwrap();
    super::super::super::audit::write(
        audit.path(),
        &candidate.key,
        "AC-X-001",
        &super::super::super::Decision {
            verdict: archon_workflow::task_set_contract::JudgeDecision::Accepted,
            counterexample: "none".into(),
            reason: "feature is absent".into(),
            sampling: Some(serde_json::json!({"provenance": "audit"})),
        },
        Some(2),
    )
    .unwrap();
    assert!(super::super::super::audit::load(audit.path(), &candidate.key, "AC-X-001").is_some());
    let findings = super::cannot_pass_findings(
        &client,
        "reviewer",
        repository.path(),
        project.path(),
        &contract,
        &BTreeSet::from(["AC-X-001".into()]),
        &baseline,
        "",
        Some(audit.path()),
    )
    .await
    .unwrap();
    assert!(findings["AC-X-001"].contains("outside the repository) rejected"));
    assert_eq!(client.evidence.load(std::sync::atomic::Ordering::SeqCst), 0);
}
