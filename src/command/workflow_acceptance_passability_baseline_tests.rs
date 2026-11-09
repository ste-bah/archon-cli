#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::Ordering::SeqCst;

use super::*;
use crate::command::workflow_task_set::passability_tests::EvidenceJudge;

fn fixture(parser_error: bool) -> (tempfile::TempDir, AcceptanceContract, BaselineRuns) {
    let external = tempfile::tempdir().unwrap();
    let executable = external.path().join("tool");
    std::fs::write(&executable, "#!/bin/sh\nexit 2\n").unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    let acceptance = ["AC-X-001", "AC-X-002"].map(|id| {
        serde_json::json!({
            "id": id, "criterion": "output is valid",
            "check": {"kind": "command", "command": "tool --new-flag", "cwd": "project_root"},
            "judgment": {"verdict": "accepted", "counterexample": "", "reason": "", "host_call_id": ""}
        })
    });
    let contract = serde_json::from_value(serde_json::json!({
        "schema_version": 1,
        "prd": {"path": "fixture.md", "digest": "fixture"},
        "gap_policy": {"permitted_acceptance_ids": [], "forbidden_phrases": [], "required_fields": []},
        "acceptance": acceptance,
        "supplementary": []
    }))
    .unwrap();
    let stderr = if parser_error {
        "error: unexpected argument '--new-flag' found\nUsage: tool [OPTIONS]\n"
    } else {
        "gate refused: below required minimum\n"
    };
    let result = archon_workflow::acceptance_scratch::CheckResult {
        acceptance_id: "AC-X-002".into(),
        exit_code: Some(2),
        stderr: stderr.as_bytes().to_vec(),
        stdout: Vec::new(),
        verdict_streams: Default::default(),
        fault: None,
        quota_walk_count: 0,
        environment_note: None,
        operational_error: None,
        classification: None,
    };
    let runs = BaselineRuns {
        commit: "evidence-tree".into(),
        failures: BTreeMap::from([(
            "AC-X-002".into(),
            super::super::super::executability::CheckEvidence {
                result,
                copy_layout: None,
            },
        )]),
        environment: BTreeMap::from([("PATH".into(), external.path().display().to_string())]),
        forwarded: Vec::new(),
        reuse: Default::default(),
    };
    (external, contract, runs)
}

fn probe_finding(tasks: &Path) -> GateFinding {
    GateFinding::new(
        GateId::FreezeAcceptance,
        "check 'AC-X-001' already holds on the evidence tree",
        "AC-X-001",
        Some(tasks.join(ACCEPTANCE_CONTRACT_FILE)),
        archon_workflow::RemediationScope::CandidateArtifact,
    )
}

#[tokio::test]
#[allow(non_snake_case)]
async fn P3_12_other_findings_do_not_defer_passability_judging() {
    let (_external, contract, mut runs) = fixture(false);
    runs.reuse.insert(
        "AC-X-002".into(),
        crate::command::workflow_task_set::executability::ReuseAudit {
            reused: false,
            why: "same base inputs".into(),
            evidence: None,
            input_key: Some("stable-base-inputs".into()),
        },
    );
    let root = tempfile::tempdir().unwrap();
    let client = EvidenceJudge::default();
    let resume = FreezeResume::saving(
        crate::command::workflow_freeze_budget::FreezeBudget::unlimited(),
        true,
    );
    for _ in 0..2 {
        let mut candidate = contract.clone();
        let findings = judge_baseline_failures(
            root.path(),
            root.path(),
            root.path(),
            "",
            &client,
            &mut candidate,
            PreImplementation {
                baseline_outcomes: Default::default(),
                findings: vec![probe_finding(root.path())],
                baseline: Some(runs.clone()),
                host_unproven_reason: None,
            },
            &resume,
        )
        .await
        .unwrap();
        assert_eq!(findings.len(), 2);
        assert_eq!(
            candidate.acceptance[1].judgment.verdict,
            JudgeDecision::Refuted
        );
        assert_eq!(
            client.evidence.load(SeqCst),
            1,
            "the unchanged input digest reuses its judgment"
        );
        assert_eq!(client.plain.load(SeqCst), 0);
    }
}

#[tokio::test]
async fn without_probe_findings_parser_and_model_refutations_are_unchanged() {
    for parser_error in [true, false] {
        let (_external, mut contract, runs) = fixture(parser_error);
        let root = tempfile::tempdir().unwrap();
        let client = EvidenceJudge::default();
        let findings = judge_baseline_failures(
            root.path(),
            root.path(),
            root.path(),
            "",
            &client,
            &mut contract,
            PreImplementation {
                baseline_outcomes: Default::default(),
                findings: Vec::new(),
                baseline: Some(runs),
                host_unproven_reason: None,
            },
            &FreezeResume::none(),
        )
        .await
        .unwrap();
        assert_eq!(findings.len(), 1, "{:?}", findings);
        assert_eq!(findings[0].subject, "AC-X-002");
        assert!(
            findings[0]
                .text
                .starts_with(&format!("check 'AC-X-002': {CANNOT_PASS}"))
        );
        assert_eq!(
            contract.acceptance[1].judgment.verdict,
            JudgeDecision::Refuted
        );
        assert_eq!(client.evidence.load(SeqCst), usize::from(!parser_error));
    }
}

fn host_text(rule: &str, reason: &str) -> String {
    let evidence = Evidence {
        commit: "base".into(),
        exit_code: Some(1),
        stderr: String::new(),
        stdout: String::new(),
        reused: false,
        reuse_why: None,
        evidence_reference: None,
    };
    evidence::host_finding_text("check", CANNOT_PASS, &evidence, rule, reason)
}
#[test]
#[allow(non_snake_case)]
fn P3_10_cargo_multiple_names_lists_every_rejected_invocation() {
    let (rule, reason) = usage::parser_diagnosis(
        "cargo",
        Path::new("/tools/cargo"),
        "error: unexpected argument 'b'",
        "set -e; cargo test --lib a b; cargo test -p package --test target c d; cargo test single",
    );
    assert!(rule.contains("'a' 'b'") && rule.contains("'c' 'd'"));
    assert!(!rule.contains("single"));
    assert_eq!(
        reason,
        "`cargo test` takes one TESTNAME before `--`; put names after `-- --exact` or run one `cargo test` per name"
    );
    assert!(host_text(&rule, &reason).contains("the host found"));
    assert!(!host_text(&rule, &reason).contains("host judge found"));
}
#[test]
#[allow(non_snake_case)]
fn P3_11_other_parser_errors_explain_repeated_form_and_host_provenance() {
    for (program, exit) in [("pytest", 4), ("cargo-nextest", 96)] {
        let (rule, reason) = usage::parser_diagnosis(
            program,
            Path::new("/tools/runner"),
            "error: unrecognized argument '--bad'",
            "runner --bad; runner --bad",
        );
        assert!(
            reason.contains("every other invocation with the same form is rejected the same way"),
            "exit {exit}"
        );
        assert!(rule.contains("rejected the check's command line"));
        assert!(host_text(&rule, &reason).contains("the host found"));
    }
}
#[test]
#[allow(non_snake_case)]
fn P3_15_passability_instruction_refutes_absent_unrequested_setup() {
    assert!(INSTRUCTION.contains("A failure to locate the check's own project, manifest, package, target or test file that the base tree lacks and no covered requirement asks to add is the check's own setup: refute"));
}

#[tokio::test]
#[allow(non_snake_case)]
async fn P3_16_reverify_call_uses_same_absent_base_setup_rule() {
    let (_external, mut contract, mut runs) = fixture(false);
    let root = tempfile::tempdir().unwrap();
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "test@example.invalid"],
        vec!["config", "user.name", "test"],
        vec!["commit", "-qm", "base", "--allow-empty"],
    ] {
        assert!(
            std::process::Command::new("git")
                .arg("-C")
                .arg(root.path())
                .args(args)
                .status()
                .unwrap()
                .success()
        );
    }
    runs.commit = "HEAD".into();
    runs.failures.get_mut("AC-X-002").unwrap().result.stderr =
        b"package ID specification `missing` did not match any packages".to_vec();
    contract.acceptance[1].check = archon_workflow::task_set_contract::AcceptanceCheck::Command {
        command: "cargo test -p missing".into(),
        cwd: archon_workflow::task_set_contract::TrustedCwd::RepoRoot,
    };
    let client = EvidenceJudge::default();
    // This is the same entry point and evidence shape used by reverify.
    let findings = judge_baseline_failures(
        root.path(),
        root.path(),
        root.path(),
        "",
        &client,
        &mut contract,
        PreImplementation {
            baseline_outcomes: Default::default(),
            findings: Vec::new(),
            baseline: Some(runs),
            host_unproven_reason: None,
        },
        &FreezeResume::none(),
    )
    .await
    .unwrap();
    assert_eq!(findings.len(), 1);
    assert!(
        findings[0].text.contains("cannot pass as written")
            && findings[0].text.contains("the host found")
    );
    assert_eq!(
        contract.acceptance[1].judgment.verdict,
        JudgeDecision::Refuted
    );
    assert_eq!(client.evidence.load(SeqCst), 0);
}

#[path = "workflow_acceptance_passability_baseline_legacy_tests.rs"]
mod legacy;

#[test]
#[allow(non_snake_case)]
fn P3_1_repaired_unproven_names_the_repair() {
    let why = crate::command::workflow_task_set::executability::test_unproven_after_repairs(
        "site unavailable",
        vec!["fresh copy".into()],
    );
    assert!(why.contains("after repairing its own environment"));
}
#[test]
#[allow(non_snake_case)]
fn P3_2_unrepaired_unproven_names_only_actual_cause() {
    for cause in [
        "policy refused",
        "site unavailable",
        "no pre-implementation tree",
        "deferred by budget",
    ] {
        let text = crate::command::workflow_task_set::executability::test_unproven_after_repairs(
            cause,
            Vec::new(),
        );
        assert!(text.contains(cause));
        assert!(!text.contains("after repairing"));
        let finding = super::super::super::coverage_gate::unproven_finding_text("check", &text);
        assert!(finding.contains(cause) && !finding.contains("after repairing"));
    }
}

use super::super::setup::diagnose;

fn repository() -> tempfile::TempDir {
    let repo = tempfile::tempdir().unwrap();
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "test@example.invalid"],
        vec!["config", "user.name", "test"],
    ] {
        assert!(setup_git(repo.path(), &args).is_some());
    }
    std::fs::create_dir_all(repo.path().join("tests")).unwrap();
    std::fs::create_dir_all(repo.path().join("src/bin")).unwrap();
    std::fs::write(
        repo.path().join("Cargo.toml"),
        "[package]\nname='existing'\nversion='0.1.0'\n",
    )
    .unwrap();
    std::fs::write(repo.path().join("tests/real.rs"), "").unwrap();
    std::fs::write(repo.path().join("src/bin/real.rs"), "").unwrap();
    assert!(setup_git(repo.path(), &["add", "."]).is_some());
    assert!(setup_git(repo.path(), &["commit", "-qm", "base"]).is_some());
    repo
}
fn case(output: &str, covers: Vec<String>) -> (AcceptanceContract, Candidate) {
    let contract: AcceptanceContract = serde_json::from_value(serde_json::json!({
        "schema_version":1,"prd":{"path":"requirements.md","digest":"d"},
        "gap_policy":{"permitted_acceptance_ids":[],"forbidden_phrases":[],"required_fields":[]},
        "acceptance":[{"id":"check","criterion":"works","covers":covers,"check":{"kind":"command","command":"cargo test -p missing --test missing","cwd":"repo_root"},"judgment":{"verdict":"accepted","counterexample":"","reason":"","host_call_id":""}}],"supplementary":[]
    })).unwrap();
    let candidate = Candidate {
        id: "check".into(),
        key: "key".into(),
        shown: serde_json::json!({}),
        evidence: Evidence {
            commit: "HEAD".into(),
            exit_code: Some(1),
            stderr: output.into(),
            stdout: String::new(),
            reused: false,
            reuse_why: None,
            evidence_reference: None,
        },
    };
    (contract, candidate)
}
#[test]
#[allow(non_snake_case)]
fn P3_13_absent_setup_is_refuted_unless_a_covered_requirement_adds_it() {
    let repo = repository();
    for output in [
        "package ID specification `missing` did not match any packages",
        "no test target named `missing`",
        "no bin target named `missing`",
        "could not find `Cargo.toml`",
        "file or directory not found: missing",
    ] {
        let (mut contract, mut candidate) = case(output, vec!["R".into()]);
        if output.contains("file or directory") {
            contract.acceptance[0].check =
                archon_workflow::task_set_contract::AcceptanceCheck::Command {
                    command: "pytest missing".into(),
                    cwd: archon_workflow::task_set_contract::TrustedCwd::RepoRoot,
                };
        }
        // Select a nested working directory for the missing-manifest case.
        if output.contains("Cargo.toml") {
            contract.acceptance[0].check =
                archon_workflow::task_set_contract::AcceptanceCheck::Command {
                    command: "cd missing; cargo test".into(),
                    cwd: archon_workflow::task_set_contract::TrustedCwd::RepoRoot,
                };
        }
        assert!(
            diagnose(
                &mut candidate,
                &contract,
                &BTreeMap::new(),
                repo.path(),
                repo.path(),
                "HEAD"
            )
            .is_some()
                == !(output.contains("target named") || output.contains("file or directory")),
            "{output}"
        );
        let name = if output.contains("Cargo.toml") {
            "Cargo.toml"
        } else {
            "missing"
        };
        let requirements = [("R".into(), format!("Add {name} to the deliverable"))].into();
        assert!(
            diagnose(
                &mut candidate,
                &contract,
                &requirements,
                repo.path(),
                repo.path(),
                "HEAD"
            )
            .is_none()
        );
        assert!(
            candidate.shown["base_setup"]["fact"]
                .as_str()
                .unwrap()
                .contains("requirement R says to add it")
        );
    }
}
#[test]
#[allow(non_snake_case)]
fn P3_14_existing_base_setup_is_not_this_rule() {
    let repo = repository();
    for output in [
        "package ID specification `existing` did not match any packages",
        "no test target named `real`",
        "no bin target named `real`",
        "could not find `Cargo.toml`",
        "file or directory not found: tests/real.rs",
        "file or directory not found: 'tests/real.rs'",
    ] {
        let (mut contract, mut candidate) = case(output, vec![]);
        contract.acceptance[0].check =
            archon_workflow::task_set_contract::AcceptanceCheck::Command {
                command: "cargo test -p existing".into(),
                cwd: archon_workflow::task_set_contract::TrustedCwd::RepoRoot,
            };
        assert!(
            diagnose(
                &mut candidate,
                &contract,
                &BTreeMap::new(),
                repo.path(),
                repo.path(),
                "HEAD"
            )
            .is_none(),
            "{output}"
        );
    }
}

fn setup_git(repository: &Path, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(args)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

#[path = "workflow_acceptance_passability_review_tests.rs"]
mod review;
