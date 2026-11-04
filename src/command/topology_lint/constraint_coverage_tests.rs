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
            format!("# TASK-X-001\n\n```yaml\ntask_id: TASK-X-001\nprd: PRD-X\ntitle: \"t\"\ncomplexity: small\nstatus: pending\ndepends_on: []\nblocks: []\nimplements: [{ids}]\nsource_sections: ['0']\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n```\n"),
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

#[test]
fn section_coverage_repairs_old_skeleton_and_accepts_draft_citations() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("PRD-X");
    fs::create_dir(&root).unwrap();
    fs::write(
        dir.path().join("PRD-X.md"),
        "## 7. Widgets\n### 7.1 Small\nRequired widgets.\n## 8 Non-goals\nExcluded.\n",
    )
    .unwrap();
    let path = root.join("task-skeleton.json");
    let mut skeleton: serde_json::Value = serde_json::json!({"schema_version":1,"acceptance_digest":"x","tasks":[{"task_id":"TASK-X-001","file_name":"TASK-X-001.md"}]});
    fs::write(&path, serde_json::to_vec(&skeleton).unwrap()).unwrap();
    let findings = policy_findings(Some(&root));
    assert_eq!(findings.len(), 1);
    assert_eq!(
        findings[0].identity.as_ref().unwrap().code,
        "uncited_section"
    );
    assert_eq!(findings[0].subject, "7.1");
    assert_eq!(
        findings[0].remediation_scope,
        archon_workflow::RemediationScope::Skeleton
    );
    assert_eq!(findings[0].source_path, path);
    skeleton["tasks"][0]["source_sections"] = serde_json::json!(["7.1"]);
    fs::write(&path, serde_json::to_vec(&skeleton).unwrap()).unwrap();
    assert!(policy_findings(Some(&root)).is_empty());
    skeleton["tasks"][0]["source_sections"] = serde_json::json!(["7.1", "404"]);
    fs::write(&path, serde_json::to_vec(&skeleton).unwrap()).unwrap();
    let findings = policy_findings(Some(&root));
    assert_eq!(findings.len(), 1);
    assert_eq!(
        findings[0].identity.as_ref().unwrap().code,
        "unknown_section"
    );
}

fn p5_fixture() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("PRD-X");
    fs::create_dir(&root).unwrap();
    fs::write(
        dir.path().join("PRD-X.md"),
        "- REQ-X-001: work\n- QX-001: bound work\n",
    )
    .unwrap();
    (dir, root)
}
fn p5_skeleton(root: &Path, ids: &[&str]) {
    let skeleton = serde_json::json!({"schema_version":1,"acceptance_digest":"x","tasks":[{"task_id":"TASK-X-001","file_name":"TASK-X-001.md","implements":ids,"source_sections":["0"]}]});
    fs::write(
        root.join("task-skeleton.json"),
        serde_json::to_vec(&skeleton).unwrap(),
    )
    .unwrap();
}
#[test]
#[allow(non_snake_case)]
fn P5_5_coverage_shares_constraint_identity() {
    let (_dir, root) = p5_fixture();
    p5_skeleton(&root, &["REQ-X-001"]);
    let findings = policy_findings(Some(&root));
    assert_eq!(findings.len(), 1);
    assert_eq!(
        findings[0].identity.as_ref().unwrap(),
        &archon_workflow::defect::DeterministicDefect::new(
            "unowned_constraint",
            "QX-001",
            "implements"
        )
    );
    assert_eq!(
        findings[0].remediation_scope,
        archon_workflow::RemediationScope::Skeleton
    );
}
#[test]
#[allow(non_snake_case)]
fn P5_32_unclaimed_remedy() {
    let (_dir, root) = p5_fixture();
    p5_skeleton(&root, &["QX-001"]);
    let findings = policy_findings(Some(&root));
    assert_eq!(findings.len(), 1);
    assert!(
        findings[0]
            .text
            .ends_with("add it to the implements of at least one task in the skeleton")
    );
}
#[test]
#[allow(non_snake_case)]
fn P5_33_unknown_remedy() {
    let (_dir, root) = p5_fixture();
    p5_skeleton(&root, &["QX-001", "REQ-X-001", "QX-999"]);
    let findings = policy_findings(Some(&root));
    assert_eq!(findings.len(), 1);
    assert!(findings[0].text.ends_with(
        "remove it from that task's skeleton implements or correct it to a host-listed id"
    ));
}
#[test]
#[allow(non_snake_case)]
fn P5_34_skeleton_claims_are_authoritative() {
    let (_dir, root) = p5_fixture();
    p5_skeleton(&root, &["QX-001", "REQ-X-001"]);
    // A stale body lacks both claims and citations; skeleton claims still govern coverage.
    fs::write(root.join("TASK-X-001.md"),"# TASK-X-001\n```yaml\ntask_id: TASK-X-001\ntitle: stale\ncomplexity: small\nstatus: pending\ndepends_on: []\nblocks: []\nimplements: []\n```\n").unwrap();
    assert!(policy_findings(Some(&root)).is_empty());
    fs::write(root.join("task-skeleton.json"), "invalid skeleton").unwrap();
    assert!(
        policy_findings(Some(&root)).is_empty(),
        "an existing unreadable skeleton cannot fall back to body claims"
    );
}
#[test]
#[allow(non_snake_case)]
fn P5_35_unparseable_body_cannot_erase_claims() {
    let (_dir, root) = p5_fixture();
    p5_skeleton(&root, &["QX-001", "REQ-X-001"]);
    fs::write(root.join("TASK-X-001.md"), "invalid body").unwrap();
    assert!(policy_findings(Some(&root)).is_empty());
    fs::remove_file(root.join("task-skeleton.json")).unwrap();
    assert!(
        policy_findings(Some(&root)).is_empty(),
        "partial body claims cannot prove a gap"
    );
    fs::write(root.join("TASK-X-002.md"), "invalid body").unwrap();
    fs::write(root.join("TASK-X-001.md"), "# TASK-X-001\n```yaml\ntask_id: TASK-X-001\nprd: PRD-X\ntitle: stale\ncomplexity: small\nstatus: pending\ndepends_on: []\nblocks: []\nimplements: [QX-999]\nsource_sections: ['0']\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n```\n").unwrap();
    let findings = policy_findings(Some(&root));
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(findings[0].text.contains("QX-999"));
}
#[test]
#[allow(non_snake_case)]
fn P5_36_report_constraint_counts_and_observe_exit() {
    let (_dir, root) = p5_fixture();
    p5_skeleton(&root, &["REQ-X-001"]);
    let report = section_with_mode(Some(&root), archon_core::config::GateMode::Observe);
    assert!(report.contains("1 constraint(s), 0 claimed."), "{report}");
    assert!(report.contains("observe: exits 0"));
    assert!(!report.contains("EXITS NON-ZERO"));
    let off = section_with_mode(Some(&root), archon_core::config::GateMode::Off);
    assert!(!off.contains("enforce:") && !off.contains("exit"), "{off}");
    let enforce = section_with_mode(Some(&root), archon_core::config::GateMode::Enforce);
    assert!(enforce.contains("enforce: policy findings exit non-zero"));
}
