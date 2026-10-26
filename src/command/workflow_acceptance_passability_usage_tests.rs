use std::collections::BTreeMap;

use super::*;

fn runs(stderr: &str) -> BaselineRuns {
    let result = archon_workflow::acceptance_scratch::CheckResult {
        fault: None,
        acceptance_id: "AC-X-001".into(),
        exit_code: Some(1),
        quota_walk_count: 0,
        stdout: Vec::new(),
        stderr: stderr.as_bytes().to_vec(),
        environment_note: None,
        operational_error: None,
        classification: None,
    };
    BaselineRuns {
        reuse: BTreeMap::new(),
        commit: "tree".into(),
        failures: BTreeMap::from([(
            "AC-X-001".into(),
            super::super::super::executability::CheckEvidence {
                result,
                copy_layout: None,
            },
        )]),
        environment: BTreeMap::new(),
        forwarded: Vec::new(),
    }
}

#[test]
fn recognizes_external_parser_errors_and_requires_both_output_signals() {
    let repository = tempfile::tempdir().unwrap();
    let cargo = runs(
        "error: unexpected argument '--exact' found\nUsage: cargo test [OPTIONS] [TESTNAME] [-- [ARGS]...]\n",
    );
    assert_eq!(
        external_usage_error(&cargo, "AC-X-001", repository.path()),
        Some((
            "cargo".into(),
            "error: unexpected argument '--exact' found".into()
        ))
    );

    let python = runs("usage: tool [options]\nerror: unrecognized arguments: --x\n");
    assert_eq!(
        external_usage_error(&python, "AC-X-001", repository.path()),
        Some(("tool".into(), "error: unrecognized arguments: --x".into()))
    );

    let plain = runs("test result: FAILED. 2 failed; 0 passed\n");
    assert_eq!(
        external_usage_error(&plain, "AC-X-001", repository.path()),
        None
    );
    let usage_only = runs("Usage: cargo test [OPTIONS]\n");
    assert_eq!(
        external_usage_error(&usage_only, "AC-X-001", repository.path()),
        None
    );
}

#[test]
fn repository_binaries_and_truncated_usage_excerpts_stay_on_the_judge_path() {
    let repository = tempfile::tempdir().unwrap();
    std::fs::write(
        repository.path().join("Cargo.toml"),
        "[package]\nname = \"fixture-core\"\nversion = \"0.1.0\"\n\n[[bin]]\nname = \"tool\"\npath = \"src/main.rs\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(repository.path().join("crates/member")).unwrap();
    std::fs::write(
        repository.path().join("crates/member/Cargo.toml"),
        "[package]\nname = \"workspace-tool\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(
        repository.path().join("package.json"),
        "{\"name\":\"fixture-js\",\"bin\":{\"js-tool\":\"bin/tool.js\"}}",
    )
    .unwrap();
    std::fs::write(
        repository.path().join("pyproject.toml"),
        "[project.scripts]\npython-tool = \"fixture:main\"\n",
    )
    .unwrap();
    let declared = runs("error: unexpected argument '--x'\nUsage: tool [OPTIONS]\n");
    assert_eq!(
        external_usage_error(&declared, "AC-X-001", repository.path()),
        None
    );
    for (program, error) in [
        ("fixture-core", "error: invalid option --x"),
        ("workspace-tool", "error: unexpected argument '--x'"),
        ("js-tool", "error: unrecognized arguments: --x"),
        ("python-tool", "error: unrecognized arguments: --x"),
    ] {
        let output = runs(&format!("{error}\nUsage: {program} [OPTIONS]\n"));
        assert_eq!(
            external_usage_error(&output, "AC-X-001", repository.path()),
            None,
            "repository-declared binary {program} stays on the judge path"
        );
    }

    let truncated = runs(&format!(
        "Usage: cargo test [OPTIONS]\n{}\nerror: unexpected argument '--exact' found\n",
        "leading diagnostic\n".repeat(140)
    ));
    assert_eq!(
        external_usage_error(&truncated, "AC-X-001", repository.path()),
        None,
        "the bounded tail omits the Usage line, so the judge decides"
    );
}

#[tokio::test]
async fn a_declared_binary_reaches_the_reauthor_passability_judge() {
    let repository = tempfile::tempdir().unwrap();
    std::fs::write(
        repository.path().join("Cargo.toml"),
        "[package]\nname = \"fixture-core\"\nversion = \"0.1.0\"\n\n[[bin]]\nname = \"tool\"\npath = \"src/main.rs\"\n",
    )
    .unwrap();
    let contract: AcceptanceContract = serde_json::from_value(serde_json::json!({
        "schema_version": 1,
        "prd": {"path": "fixture.md", "digest": "fixture"},
        "gap_policy": {"permitted_acceptance_ids": [], "forbidden_phrases": [], "required_fields": []},
        "acceptance": [{
            "id": "AC-X-001", "criterion": "output is valid",
            "check": {"kind": "command", "command": "true", "cwd": "project_root"},
            "judgment": {"verdict": "accepted", "counterexample": "", "reason": "", "host_call_id": ""}
        }],
        "supplementary": []
    }))
    .unwrap();
    let evidence = runs("error: unexpected argument '--x'\nUsage: tool [OPTIONS]\n");
    let judge = super::super::super::passability_tests::EvidenceJudge::default();
    let findings = super::cannot_pass_findings(
        &judge,
        "sonnet",
        repository.path(),
        &contract,
        &BTreeSet::from(["AC-X-001".into()]),
        &evidence,
        "",
        None,
    )
    .await
    .unwrap();
    assert!(findings.is_empty());
    assert_eq!(judge.evidence.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(
        contract.acceptance[0].judgment.verdict,
        JudgeDecision::Accepted
    );
}

#[test]
fn the_usage_line_nearest_the_parser_error_names_the_rejecting_program() {
    let repository = tempfile::tempdir().unwrap();
    let output = "Usage: helper-x [OPTIONS]\nstep one ok\nstep two ok\nerror: unexpected argument '--y' found\nUsage: helper-y [OPTIONS]\n";
    assert_eq!(
        external_usage_error(&runs(output), "AC-X-001", repository.path()),
        Some((
            "helper-y".into(),
            "error: unexpected argument '--y' found".into()
        ))
    );
}
