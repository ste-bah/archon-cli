//! P6 trace wording follows the skeleton's ownership of frozen fields.
use super::*;

#[test]
#[allow(non_snake_case)]
fn P6_19_trace_remedies_edit_the_skeleton_and_keep_the_PRD_read_only() {
    let dir = tempfile::tempdir().unwrap();
    let prd = write_prd(dir.path(), &["- REQ-X-001: work"]);
    let tasks = write_trace_task(
        dir.path(),
        "```yaml\ntask_id: TASK-X-001\nimplements: [REQ-X-999]\n```\n",
    );
    let report = build_report(dir.path(), &TraceOptions::new(prd, tasks)).unwrap();
    let findings = crate::command::requirement_trace::verdict::policy_findings(&report, Vec::new());
    assert_eq!(findings.len(), 2);
    for finding in findings {
        assert_eq!(
            finding.remediation_scope,
            archon_workflow::RemediationScope::Skeleton
        );
        assert!(
            finding
                .text
                .contains("frozen skeleton task's implements list")
        );
        assert!(!finding.text.contains("TASK file's implements"));
        assert!(
            !finding
                .text
                .contains("remove or correct the obligation in the PRD")
        );
    }
}

#[test]
fn numbered_section_citations_do_not_become_unknown_obligations() {
    let dir = tempfile::tempdir().unwrap();
    let prd = write_prd(dir.path(), &["## 7. Widgets", "- REQ-X-001: work"]);
    let tasks = write_trace_task(
        dir.path(),
        "```yaml\ntask_id: TASK-X-001\ntitle: Widgets\ncomplexity: low\nstatus: ready\ndepends_on: []\nblocks: []\nimplements: [REQ-X-001]\nsource_sections: ['7']\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n```\n",
    );
    let evaluation = evaluate_trace(dir.path(), &TraceOptions::new(prd, tasks)).unwrap();
    assert!(
        !evaluation
            .findings
            .iter()
            .any(|finding| finding.text.contains("section")
                || finding.text.contains("unknown obligation")),
        "{:?}",
        evaluation.findings
    );
}

pub(super) fn constraint_claim_fixture() -> (tempfile::TempDir, TraceOptions) {
    let dir = tempfile::tempdir().unwrap();
    let prd = write_prd(dir.path(), &["- QX-001: restriction"]);
    let raw = "```yaml\ntask_id: TASK-X-001\ntitle: T\ncomplexity: low\nstatus: ready\ndepends_on: []\nblocks: []\nimplements: [QX-001]\nsource_sections: ['0']\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n```\n\n## Files Expected to Change\n\n- `src/lib.rs`\n";
    let tasks = write_trace_task(dir.path(), raw);
    for args in [
        vec!["init", "-q"],
        vec![
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-qm",
            "base",
        ],
    ] {
        assert!(
            std::process::Command::new("git")
                .current_dir(dir.path())
                .args(args)
                .status()
                .unwrap()
                .success()
        );
    }
    let output = std::process::Command::new("git")
        .current_dir(dir.path())
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    archon_workflow::repository_record::write_repository_record(
        &tasks,
        &archon_workflow::repository_record::RepositoryRecordV1 {
            schema_version: archon_workflow::repository_record::REPOSITORY_RECORD_SCHEMA_VERSION,
            repository_root: dir.path().display().to_string(),
            base_commit: String::from_utf8(output.stdout).unwrap().trim().to_string(),
            decomposition_run_id: "run".into(),
            recorded_at: "now".into(),
        },
    )
    .unwrap();
    (dir, TraceOptions::new(prd, tasks))
}
