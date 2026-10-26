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

fn check(runs: &BaselineRuns, repository: &Path) -> Option<(String, PathBuf, String)> {
    check_command(runs, repository, "tool --x")
}

fn check_command(
    runs: &BaselineRuns,
    repository: &Path,
    command: &str,
) -> Option<(String, PathBuf, String)> {
    super::external_usage_error(runs, "AC-X-001", repository, repository, command)
}

#[cfg(unix)]
fn executable(dir: &Path, name: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = dir.join(name);
    std::fs::write(&path, "#!/bin/sh\nexit 2\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[cfg(unix)]
#[test]
fn only_resolved_external_programs_with_parser_errors_are_host_refuted() {
    let repository = tempfile::tempdir().unwrap();
    let external = tempfile::tempdir().unwrap();
    for name in ["jq", "poetry-tool", "runner", "tool", "worker"] {
        let _ = executable(external.path(), name);
    }
    let actual = super::external_usage_error_on_path(
        &runs("tool: error: unrecognized arguments: --new-flag\n"),
        "AC-X-001",
        repository.path(),
        repository.path(),
        "tool --new-flag",
        external.path().as_os_str(),
    );
    assert_eq!(
        actual,
        Some((
            "tool".into(),
            external.path().join("tool"),
            "tool: error: unrecognized arguments: --new-flag".into()
        )),
        "a PATH-resolved external parser refusal is the only host-refuted case"
    );

    for (name, output) in [
        (
            "repo-script",
            "usage: worker [--value VALUE]\nworker: error: unrecognized arguments: --new-flag\n",
        ),
        (
            "repo-script-external-looking",
            "tool: error: unrecognized arguments: --new-flag\nUsage: tool [OPTIONS]\n",
        ),
        (
            "cargo-implicit",
            "error: unexpected argument '--new-flag' found\nUsage: tool [OPTIONS]\n",
        ),
        (
            "poetry",
            "error: unexpected argument '--new-flag' found\nUsage: poetry-tool [OPTIONS]\n",
        ),
        (
            "scoped-npm",
            "error: unexpected argument '--new-flag' found\nUsage: tool [OPTIONS]\n",
        ),
    ] {
        let case_repository = tempfile::tempdir().unwrap();
        match name {
            "repo-script" | "repo-script-external-looking" => {
                std::fs::create_dir_all(case_repository.path().join("scripts")).unwrap();
                std::fs::write(case_repository.path().join("scripts/worker"), "").unwrap();
            }
            "cargo-implicit" => {
                std::fs::create_dir_all(case_repository.path().join("src/bin")).unwrap();
                std::fs::write(case_repository.path().join("src/bin/tool.rs"), "").unwrap();
            }
            "poetry" => std::fs::write(
                case_repository.path().join("pyproject.toml"),
                "[tool.poetry.scripts]\npoetry-tool = 'pkg:main'\n",
            )
            .unwrap(),
            "scoped-npm" => std::fs::write(
                case_repository.path().join("package.json"),
                r#"{"name":"@scope/pkg","bin":{"@scope/tool":"bin/tool.js"}}"#,
            )
            .unwrap(),
            _ => unreachable!(),
        }
        assert_eq!(
            // An unrelated PATH executable exists. The check invokes this
            // repository script through a launcher.
            super::external_usage_error_on_path(
                &runs(output),
                "AC-X-001",
                case_repository.path(),
                case_repository.path(),
                match name {
                    "repo-script" | "repo-script-external-looking" =>
                        "runner scripts/worker --new-flag",
                    "poetry" => "poetry-tool --new-flag",
                    "scoped-npm" => "tool --new-flag",
                    _ => "tool --new-flag",
                },
                external.path().as_os_str(),
            ),
            None,
            "repository programs stay on the judge path: {name}"
        );
    }
    let nested = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(nested.path().join("crates/pkg/src/bin/runner")).unwrap();
    std::fs::write(nested.path().join("crates/pkg/src/bin/runner/main.rs"), "").unwrap();
    assert_eq!(
        super::external_usage_error_on_path(
            &runs("error: unexpected argument '--x' found\nUsage: runner [OPTIONS]\n"),
            "AC-X-001",
            nested.path(),
            nested.path(),
            "runner --x",
            external.path().as_os_str(),
        ),
        None,
        "an implicit Cargo directory binary stays on the judge path"
    );

    assert_eq!(
        check(
            &runs("Usage: not-on-path [OPTIONS]\nerror: unexpected argument '--x'\n"),
            repository.path()
        ),
        None,
        "an unresolved name stays on the judge path"
    );
    assert_eq!(
        check_command(
            &runs("Usage: runner scripts/worker [OPTIONS]\nerror: unexpected argument '--x'\n"),
            repository.path(),
            "runner scripts/worker --x",
        ),
        None,
        "a usage line naming an interpreter and repository script identifies the script"
    );
    assert_eq!(
        super::external_usage_error_on_path(
            &runs("error: unexpected argument '--x' found\nUsage: runner -m pkg [OPTIONS]\n"),
            "AC-X-001",
            repository.path(),
            repository.path(),
            "runner -m pkg --x",
            external.path().as_os_str(),
        ),
        None,
        "a usage line naming an interpreter and module identifies a repository program"
    );
    assert_eq!(
        super::external_usage_error_on_path(
            &runs(
                "jq: error: Could not open file out.json: No such file or directory\nUsage: jq [OPTIONS]\n"
            ),
            "AC-X-001",
            repository.path(),
            repository.path(),
            "jq --arg value x out.json",
            external.path().as_os_str(),
        ),
        None,
        "a file error from an external program is not an argument refusal"
    );
    assert_eq!(
        check(
            &runs("the product reported an invalid option value for this field\n"),
            repository.path()
        ),
        None,
        "a phrase inside product text is not a parser-line shape"
    );
    let distant = format!(
        "tool: error: unrecognized arguments: --new-flag\n{}Usage: tool [OPTIONS]\n",
        "diagnostic\n".repeat(10)
    );
    assert_eq!(
        check(&runs(&distant), repository.path()),
        None,
        "usage more than three lines from the error is not program identity"
    );

    std::fs::write(repository.path().join("package.json"), "{").unwrap();
    assert_eq!(
        check(
            &runs("error: unexpected argument '--new-flag' found\nUsage: tool [OPTIONS]\n"),
            repository.path()
        ),
        None,
        "a malformed manifest makes the declared set incomplete"
    );
}

#[test]
fn recognizes_external_parser_errors_and_requires_both_output_signals() {
    let repository = tempfile::tempdir().unwrap();
    let cargo = runs(
        "error: unexpected argument '--exact' found\n\n  tip: to pass '--exact' as a value, use '-- --exact'\n\nUsage: cargo test [OPTIONS] [TESTNAME] [-- [ARGS]...]\n\nFor more information, try '--help'.\n",
    );
    assert_eq!(
        check(&cargo, repository.path()),
        std::env::var_os("PATH")
            .and_then(|path| super::resolve_executable("cargo", &path))
            .and_then(|path| {
                Some((
                    "cargo".into(),
                    path,
                    "error: unexpected argument '--exact' found".into(),
                ))
            })
    );

    let python = runs("usage: tool [options]\nerror: unrecognized arguments: --x\n");
    assert_eq!(
        check(&python, repository.path()),
        None,
        "an unresolved parser program stays on the judge path"
    );

    let plain = runs("test result: FAILED. 2 failed; 0 passed\n");
    assert_eq!(check(&plain, repository.path()), None);
    let usage_only = runs("Usage: cargo test [OPTIONS]\n");
    assert_eq!(check(&usage_only, repository.path()), None);
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
    assert_eq!(check(&declared, repository.path()), None);
    for (program, error) in [
        ("fixture-core", "error: invalid option --x"),
        ("workspace-tool", "error: unexpected argument '--x'"),
        ("js-tool", "error: unrecognized arguments: --x"),
        ("python-tool", "error: unrecognized arguments: --x"),
    ] {
        let output = runs(&format!("{error}\nUsage: {program} [OPTIONS]\n"));
        assert_eq!(
            check(&output, repository.path()),
            None,
            "repository-declared binary {program} stays on the judge path"
        );
    }

    let truncated = runs(&format!(
        "Usage: cargo test [OPTIONS]\n{}\nerror: unexpected argument '--exact' found\n",
        "leading diagnostic\n".repeat(140)
    ));
    assert_eq!(
        check(&truncated, repository.path()),
        None,
        "the bounded tail omits the Usage line, so the judge decides"
    );
}

#[cfg(unix)]
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
        "reviewer",
        repository.path(),
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

#[cfg(unix)]
#[tokio::test]
async fn external_parser_refusal_uses_bounded_evidence_and_the_audit_gate() {
    use std::os::unix::fs::symlink;

    let repository = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let external = tempfile::tempdir().unwrap();
    let executable = executable(external.path(), "tool-real");
    let path_entry = external.path().join("tool");
    symlink(&executable, &path_entry).unwrap();
    std::fs::write(
        &executable,
        "#!/bin/sh\nprintf '%s\\n' \"error: unexpected argument '--new-flag' found\" >&2\nprintf '%s\\n' 'Usage: tool [OPTIONS]' >&2\nexit 2\n",
    )
    .unwrap();
    let output = std::process::Command::new(&executable)
        .arg("--new-flag")
        .output()
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
    assert_eq!(output.status.code(), Some(2));
    let mut baseline = runs(std::str::from_utf8(&output.stderr).unwrap());
    baseline.environment.insert(
        "PATH".into(),
        external.path().to_string_lossy().into_owned(),
    );
    let judge = super::super::super::passability_tests::EvidenceJudge::default();
    let audit = tempfile::tempdir().unwrap();
    let findings = super::cannot_pass_findings(
        &judge,
        "model-x",
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
    let finding = &findings["AC-X-001"];
    assert!(finding.contains(&format!("tool ({}", path_entry.display())));
    assert!(finding.contains("outside the repository) rejected the check's command line"));
    assert!(finding.contains("stderr:"));
    assert!(finding.contains("unexpected argument '--new-flag' found"));
    assert!(
        finding
            .contains("nothing the repository implements can change how tool parses its arguments")
    );
    assert_eq!(judge.plain.load(std::sync::atomic::Ordering::SeqCst), 0);
    let record: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            std::fs::read_dir(audit.path())
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path(),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(record["verdict"], "refuted");
}

#[path = "workflow_acceptance_passability_usage_order_tests.rs"]
mod order_tests;

#[test]
fn the_usage_line_nearest_the_parser_error_names_the_rejecting_program() {
    let repository = tempfile::tempdir().unwrap();
    let output = "Usage: helper-x [OPTIONS]\nstep one ok\nstep two ok\nerror: unexpected argument '--y' found\nUsage: helper-y [OPTIONS]\n";
    assert_eq!(
        check(&runs(output), repository.path()),
        None,
        "a nearby Usage line still needs a PATH-resolved executable"
    );
}
