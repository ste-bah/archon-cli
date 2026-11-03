use super::*;

#[test]
fn each_unowned_constraint_is_a_skeleton_repair_and_owned_constraints_are_known() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("PRD-X");
    fs::create_dir(&root).unwrap();
    fs::write(dir.path().join("PRD-X.md"), "- REQ-X-001: work\n- QX-001: restrict work\n- QX-002: restrict other work\n- QX-002: duplicate\n").unwrap();
    let task = root.join("TASK-X-001.md");
    let write_task = |ids: &str| {
        fs::write(
            &task,
            format!("# TASK-X-001\n\n```yaml\ntask_id: TASK-X-001\nprd: PRD-X\ntitle: \"t\"\ncomplexity: small\nstatus: pending\ndepends_on: []\nblocks: []\nimplements: [{ids}]\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n```\n"),
        )
        .unwrap()
    };
    write_task("REQ-X-001, QX-001");
    let findings = policy_findings(Some(&root));
    assert_eq!(findings.len(), 1, "{findings:#?}");
    let finding = &findings[0];
    assert_eq!(finding.subject, "QX-002");
    assert_eq!(
        finding.remediation_scope,
        archon_workflow::RemediationScope::Skeleton
    );
    assert_eq!(
        finding.identity.as_ref().unwrap().code,
        "unowned_constraint"
    );
    assert_eq!(finding.identity.as_ref().unwrap().subject, "QX-002");
    assert!(finding.source_path.ends_with("task-skeleton.json"));
    assert!(finding.text.contains("every task whose work it restricts"));
    write_task("REQ-X-001, QX-001, QX-002");
    assert!(policy_findings(Some(&root)).is_empty());
}
