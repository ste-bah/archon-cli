use super::*;
use archon_workflow::task_set_contract::{
    AcceptanceCheck, AcceptanceContract, AcceptanceCriterion, GapPolicy, JudgeDecision,
    JudgeVerdict, PrdIdentity,
};
use std::io::Write;

#[cfg(unix)]
#[test]
fn missing_typed_program_uses_existing_product_and_site_classification() {
    let site = tempfile::tempdir().unwrap();
    let tasks = site.path().join("tasks");
    std::fs::create_dir(&tasks).unwrap();
    let absolute_product = site
        .path()
        .join("absent-product")
        .to_string_lossy()
        .replace('\\', "/");
    for (command, absent) in [
        ("./absent-product", false),
        ("absent-product", true),
        ("absent-product-extension", true),
        ("absent-site-program-s7", true),
        (absolute_product.as_str(), true),
    ] {
        let artifact_path = if command == "absent-product-extension" {
            "bin/absent-product-extension.exe".into()
        } else if command == "absent-product" {
            "bin/absent-product".into()
        } else if command == absolute_product {
            absolute_product.clone()
        } else {
            "out.json".into()
        };
        let command = if command == absolute_product {
            format!("'{}'", command.replace('\'', "'\"'\"'"))
        } else {
            command.to_string()
        };
        let mut child = std::process::Command::new(archon_shell::resolve_posix_shell())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(command.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        let result: CheckResult = serde_json::from_value(serde_json::json!({"acceptance_id":"A-1","exit_code":output.status.code(),"stdout":output.stdout,"stderr":output.stderr,"operational_error":null})).unwrap();
        let contract = AcceptanceContract {
            schema_version: 1,
            prd: PrdIdentity {
                path: "source.md".into(),
                digest: "digest".into(),
            },
            gap_policy: GapPolicy {
                permitted_acceptance_ids: Default::default(),
                forbidden_phrases: Vec::new(),
                required_fields: Vec::new(),
            },
            acceptance: vec![AcceptanceCriterion {
                id: "A-1".into(),
                criterion: "evidence".into(),
                covers: Vec::new(),
                gap_permitted: false,
                judgment: JudgeVerdict {
                    verdict: JudgeDecision::Accepted,
                    reason: "checks evidence".into(),
                    counterexample: "missing evidence".into(),
                    sampling: None,
                    host_call_id: "judge".into(),
                },
                check: AcceptanceCheck::Floor {
                    contract: Box::new(
                        archon_workflow::task_universe::WorkflowV2DeliverableContract {
                            kind: "evidence".into(),
                            artifact_path,
                            typed_verifier_command: Some(command.clone()),
                            ..Default::default()
                        },
                    ),
                },
            }],
            supplementary: Vec::new(),
        };
        let findings = crash_findings_at(&contract, [&result], None);
        assert!(
            findings.is_empty(),
            "a missing program is not a script defect"
        );
        let at = super::verdict::Context::on_host_path(&contract);
        let cause = super::verdict::no_verdict_cause(&command, &result, &at);
        if absent {
            let (reason, cause) = cause.expect("site program gives no verdict");
            assert_eq!(cause, UnprovenCause::ProgramAbsent);
            let probe = HostProbe::for_task_set(site.path(), &tasks);
            let finding = super::silent::settle_as(
                &probe,
                BaselineOrigin::RecordedBase,
                "test-base",
                &contract,
                &result,
                (&reason, cause),
                &at,
            )
            .expect("author route, no pause");
            assert!(finding.contains("Check the criterion with what the site provides"));
            assert!(probe.take_unproven_causes().is_empty());
        } else {
            assert!(
                cause.is_none(),
                "the missing product is implementation work"
            );
        }
    }
}

/// Real process output goes through the host callback, branch enforcement and
/// the production wave aggregator; no model reply supplies a triage verdict.
#[tokio::test]
async fn missing_typed_program_host_wave_records_unverified_and_keeps_product_failures() {
    use archon_workflow::v2::deliverable_contract::ContractRoots;
    use archon_workflow::v2::{WorkflowV2BranchOutcome, WorkflowV2Result, WorkflowV2Status};
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("out.txt"), "evidence").unwrap();
    let failing_program = format!(
        "'{}' -c 'exit 1'",
        archon_shell::resolve_posix_shell()
            .to_string_lossy()
            .replace('\\', "/")
            .replace('\'', "'\"'\"'")
    );
    for (command, absent) in [
        ("absent-site-program-s7", true),
        (failing_program.as_str(), false),
        ("./absent-product", false),
        ("exit 1", false),
        ("exit 126", false),
        ("exit 127", false),
    ] {
        let started = std::time::Instant::now();
        let contracts = BTreeMap::from([(
            "floor".into(),
            (
                ContractRoots::project_only(directory.path().to_string_lossy()),
                vec![
                    serde_json::json!({"kind":"evidence","artifact_path":"out.txt","typed_verifier_command":command}),
                ],
            ),
        )]);
        let original = contracts.clone();
        let inspection = format!(
            "test -s '{}'",
            directory
                .path()
                .join("out.txt")
                .to_string_lossy()
                .replace('\\', "/")
                .replace('\'', "'\"'\"'")
        );
        let inspected = std::process::Command::new(archon_shell::resolve_posix_shell())
            .arg("-c")
            .arg(&inspection)
            .output()
            .unwrap();
        assert!(inspected.status.success());
        let mut verified = WorkflowV2Result::accepted("evidence file inspected");
        verified
            .commands_run
            .push(archon_workflow::WorkflowV2CommandRecord {
                kind: archon_workflow::WorkflowV2CommandKind::Inspect,
                command: inspection,
                status: archon_workflow::WorkflowV2CommandStatus::Succeeded,
                exit_code: inspected.status.code(),
                output_summary: String::from_utf8(inspected.stdout).unwrap(),
                pre_existing: false,
            });
        let mut outcomes = vec![WorkflowV2BranchOutcome {
            item_id: "floor".into(),
            role: "verifier".into(),
            status: WorkflowV2Status::Accepted,
            result: Some(verified),
            error: None,
            failure_kind: None,
            item_input_hash: None,
            completion_evidence: Vec::new(),
        }];
        archon_workflow::v2::verification::enforce_declared_contracts_classified(
            &mut outcomes,
            &contracts,
            None,
            Some(missing_typed_program),
        )
        .await;
        assert_eq!(contracts, original);
        if !absent {
            assert_eq!(
                outcomes[0].result.as_ref().unwrap().data["verification_failure_class"],
                "declared_contract_violation"
            );
        }
        let call = serde_json::from_value(serde_json::json!({
            "id":"verification-wave-1", "method":"fanout", "options":{"role":"verifier"}
        }))
        .unwrap();
        let report = archon_workflow::v2::call_data::result_from_fanout_report(
            &call,
            archon_workflow::WorkflowV2FanoutReport {
                outcomes,
                max_parallelism: 1,
                peak_parallelism: 1,
                cancelled: false,
            },
        );
        let result = report.outcomes[0].result.as_ref().unwrap();
        if absent {
            assert_eq!(report.result.status, WorkflowV2Status::Accepted);
            assert_eq!(report.outcomes[0].status, WorkflowV2Status::Accepted);
            assert!(report.result.residual_gaps.is_empty());
            assert_eq!(result.data["declared_contract_verification"], "unverified");
            assert_eq!(result.data["declared_contracts_verified"], 0);
            assert_eq!(
                result.data["declared_contracts_unverified"],
                serde_json::json!([
                    "unverified: typed command unavailable (absent-site-program-s7 not found)"
                ])
            );
            let path = directory.path().join("verification-report.json");
            std::fs::write(&path, serde_json::to_vec(&report.result).unwrap()).unwrap();
            let stored: serde_json::Value =
                serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
            assert!(
                stored
                    .to_string()
                    .contains("unverified: typed command unavailable")
            );
        } else {
            assert_eq!(report.result.status, WorkflowV2Status::NeedsReview);
            assert_eq!(result.data["declared_contract_verification"], "failed");
            // The normalizer may refine the class; this exact gap is what the
            // unchanged lifecycle driver dispatches to implementation repair.
            assert!(
                result
                    .residual_gaps
                    .iter()
                    .any(|gap| gap.id == "declared_contract_verification_failed")
            );
        }
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
    }
}

#[cfg(unix)] // Requires POSIX executable permission bits, which the other platform does not enforce.
#[test]
fn missing_typed_program_permission_denied_is_reported_literally() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().unwrap();
    let program = directory.path().join("not-executable");
    std::fs::write(&program, "exit 0\n").unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o600)).unwrap();
    let program = program.to_string_lossy().to_string();
    let command = format!("'{}'", program.replace('\'', "'\"'\"'"));
    let environment =
        archon_workflow::acceptance_check_environment::CommandEnvironment::capture(None).unwrap();
    let mut process = environment.command(archon_shell::resolve_posix_shell());
    let variables = process
        .get_envs()
        .filter_map(|(key, value)| Some((key.to_str()?.to_string(), value?.to_str()?.to_string())))
        .collect();
    let output = process.arg("-c").arg(&command).output().unwrap();
    assert_eq!(output.status.code(), Some(126));
    let result: CheckResult = serde_json::from_value(serde_json::json!({
        "acceptance_id":"A-1", "exit_code":output.status.code(), "stdout":output.stdout,
        "stderr":output.stderr, "operational_error":null
    }))
    .unwrap();
    assert_eq!(
        missing_typed_program(&command, &result, &variables),
        Some(format!("{program} not executable"))
    );
}
