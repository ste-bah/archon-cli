use super::*;
use archon_workflow::task_set_contract::{
    AcceptanceCheck, AcceptanceCriterion, JudgeVerdict, TrustedCwd,
};
use archon_workflow::task_skeleton::{FrozenTask, TaskSkeleton};

#[path = "workflow_decompose_reverify_routing_tests.rs"]
mod remediation_routing;

fn entry(id: &str, covers: &[&str], verdict: JudgeDecision) -> AcceptanceCriterion {
    AcceptanceCriterion {
        id: id.into(),
        criterion: "The check proves the obligation".into(),
        check: AcceptanceCheck::Command {
            command: "true".into(),
            cwd: TrustedCwd::ProjectRoot,
        },
        gap_permitted: false,
        judgment: JudgeVerdict {
            verdict,
            counterexample: "none".into(),
            reason: "frozen judgment".into(),
            host_call_id: "freeze-judge".into(),
            sampling: None,
        },
        covers: covers.iter().map(|cover| (*cover).into()).collect(),
    }
}

fn contract() -> AcceptanceContract {
    AcceptanceContract {
        schema_version: 1,
        prd: archon_workflow::task_set_contract::PrdIdentity {
            path: "PRD.md".into(),
            digest: "digest".into(),
        },
        gap_policy: archon_workflow::task_set_contract::GapPolicy {
            permitted_acceptance_ids: Default::default(),
            forbidden_phrases: Vec::new(),
            required_fields: Vec::new(),
        },
        acceptance: vec![entry("AC-X-001", &["REQ-X-001"], JudgeDecision::Accepted)],
        supplementary: vec![entry("SUP-X-002", &["REQ-X-002"], JudgeDecision::Refuted)],
    }
}

#[cfg(unix)]
#[tokio::test]
async fn reverify_report_sibling_paths_come_from_each_real_probe_check() {
    #[cfg(unix)]
    use crate::command::workflow_task_set::executability::probe_tests::trees;

    let trees = trees(&[
        (
            "AC-X-001",
            "grep -q ready ../sibling-a/fixture.txt",
            TrustedCwd::RepoRoot,
        ),
        (
            "AC-X-002",
            "grep -q ready ../sibling-b/fixture.txt",
            TrustedCwd::RepoRoot,
        ),
        (
            "AC-X-003",
            "grep -q ready src/state.txt",
            TrustedCwd::RepoRoot,
        ),
    ]);
    for name in ["sibling-a", "sibling-b"] {
        let root = trees.outside.path().join(name);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("fixture.txt"), "ready\n").unwrap();
    }
    let copies = tempfile::tempdir().unwrap();
    let resume = crate::command::workflow_freeze_budget::FreezeResume::staged("freeze-acceptance");
    let probe =
        crate::command::workflow_task_set::executability::HostProbe::for_reverify_with_limits(
            trees.set.project.path(),
            &trees.set.tasks,
            archon_tools::workflow_resource_env::CargoResourceLimits::default(),
            &resume,
        )
        .with_host_environment(crate::test_environment::probe_host())
        .with_copy_parent(copies.path().to_path_buf())
        .without_process_memo();
    let (evidence, _, _, _) = probe
        .run_reverify_copy(&trees.contract(), &trees.ids())
        .await
        .expect("real two-root reverify probe");

    let reported = |id: &str| {
        let item = evidence
            .iter()
            .find(|item| item.acceptance_id == id)
            .unwrap();
        CheckResult {
            check_id: id.into(),
            verdict: "accepted".into(),
            really_ran: true,
            setup_failure: false,
            real_failure: false,
            sibling_paths: super::reported_siblings(id, item),
            findings: Vec::new(),
            not_run_reason: None,
            regression_guard: None,
            remediation_route: None,
        }
    };
    let check_a = reported("AC-X-001");
    let check_b = reported("AC-X-002");
    let check_c = reported("AC-X-003");
    assert_eq!(check_a.sibling_paths.len(), 1);
    assert_eq!(check_a.sibling_paths[0].path, "../sibling-a");
    assert_eq!(check_b.sibling_paths.len(), 1);
    assert_eq!(check_b.sibling_paths[0].path, "../sibling-b");
    assert!(check_c.sibling_paths.is_empty());
}

#[test]
fn a_failed_check_without_an_implementing_task_keeps_the_check_as_subject() {
    // EXPECTED BEFORE RUN: no task implements this check id or covered
    // requirement, so the check id itself remains eligible for remediation.
    let skeleton = TaskSkeleton {
        schema_version: 1,
        acceptance_digest: "digest".into(),
        tasks: vec![FrozenTask {
            task_id: "TASK-X-010".into(),
            file_name: "TASK-X-010.md".into(),
            depends_on: Vec::new(),
            blocks: Vec::new(),
            implements: vec!["REQ-X-OTHER".into()],
            deliverable_contracts: Vec::new(),
        }],
    };
    let subject = check_subjects(&contract().acceptance[0], &skeleton);
    assert_eq!(subject, ["AC-X-001"]);
}

#[test]
fn only_checks_needing_remediation_produce_task_subjects() {
    // EXPECTED BEFORE RUN: an accepted check with a real failure and a
    // not_run check produce no remediation subjects; a setup failure that is
    // needs_remediation produces a check subject for check setup defects.
    let skeleton = TaskSkeleton {
        schema_version: 1,
        acceptance_digest: "digest".into(),
        tasks: vec![FrozenTask {
            task_id: "TASK-X-010".into(),
            file_name: "TASK-X-010.md".into(),
            depends_on: Vec::new(),
            blocks: Vec::new(),
            implements: vec!["REQ-X-001".into()],
            deliverable_contracts: Vec::new(),
        }],
    };
    let contract = contract();
    let accepted_failure = CheckResult {
        check_id: "AC-X-001".into(),
        verdict: "accepted".into(),
        really_ran: true,
        setup_failure: false,
        real_failure: true,
        sibling_paths: Vec::new(),
        findings: Vec::new(),
        not_run_reason: None,
        regression_guard: None,
        remediation_route: None,
    };
    let not_run = not_run_checks(&frozen_check_plan(&contract));
    assert!(
        remediation_subjects(
            &[accepted_failure, not_run[0].clone()],
            &contract,
            &skeleton
        )
        .is_empty()
    );

    let setup_failure = CheckResult {
        check_id: "AC-X-001".into(),
        verdict: "needs_remediation".into(),
        really_ran: true,
        setup_failure: true,
        real_failure: false,
        sibling_paths: Vec::new(),
        findings: vec!["invalid setup".into()],
        not_run_reason: None,
        regression_guard: None,
        remediation_route: None,
    };
    assert_eq!(
        remediation_subjects(&[setup_failure], &contract, &skeleton),
        ["AC-X-001"]
    );
}

#[test]
fn frozen_check_plan_accounts_for_nonaccepted_supplementary_entries() {
    // EXPECTED BEFORE RUN: the report plan contains every acceptance and
    // supplementary entry; Refuted entries say why freeze does not run them.
    let plan = frozen_check_plan(&contract());
    let checks = serde_json::to_value(not_run_checks(&plan)).unwrap();
    assert_eq!(plan.len(), 2);
    assert_eq!(checks.as_array().unwrap().len(), 1);
    let entry = &checks[0];
    assert_eq!(entry["check_id"], "SUP-X-002");
    assert_eq!(entry["really_ran"], false);
    assert!(
        entry["not_run_reason"]
            .as_str()
            .unwrap()
            .contains("Refuted")
    );
    assert!(
        entry["not_run_reason"]
            .as_str()
            .unwrap()
            .contains("freeze does not run")
    );
}

#[test]
fn reverify_progress_reports_each_check_in_a_two_check_plan() {
    // EXPECTED BEFORE RUN: both accepted entries get a visible start and a
    // terminal outcome with elapsed seconds, so a long check cannot hide the
    // number of checks in the reverify plan.
    let mut contract = contract();
    contract.supplementary[0].judgment.verdict = JudgeDecision::Accepted;
    let plan = frozen_check_plan(&contract);
    assert_eq!(plan.len(), 2);
    let mut progress = CheckProgress::new(plan.len());
    let lines = plan
        .iter()
        .enumerate()
        .map(|(index, (id, _, _))| {
            progress.start(index + 1, id);
            progress.start_line(index + 1, id)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        lines,
        ["check 1/2 AC-X-001 started", "check 2/2 SUP-X-002 started"]
    );
    assert_eq!(
        progress.finish_line(1, "AC-X-001", "passed", 1.26),
        "check 1/2 AC-X-001 passed elapsed=1.3s"
    );
    assert_eq!(
        progress.finish_line(2, "SUP-X-002", "reused from cache", 0.0),
        "check 2/2 SUP-X-002 reused from cache elapsed=0.0s"
    );
}

#[test]
fn regression_guard_basis_is_reported_and_optional_for_older_entries() {
    let check = CheckResult {
        check_id: "AC-X-001".into(),
        verdict: "accepted".into(),
        really_ran: true,
        setup_failure: false,
        real_failure: false,
        sibling_paths: Vec::new(),
        findings: Vec::new(),
        not_run_reason: None,
        regression_guard: Some(
            "accepted on the stored judge verdict because it passed on the baseline".into(),
        ),
        remediation_route: None,
    };
    let report_entry = serde_json::to_value(&check).unwrap();
    assert!(
        report_entry["regression_guard"]
            .as_str()
            .unwrap()
            .contains("stored judge verdict")
    );
    assert_eq!(
        serde_json::from_value::<CheckResult>(serde_json::json!({
            "check_id": "AC-X-002",
            "verdict": "accepted",
            "really_ran": true,
            "setup_failure": false,
            "real_failure": false,
            "sibling_paths": [],
            "findings": []
        }))
        .unwrap()
        .regression_guard,
        None
    );
    assert_eq!(
        serde_json::from_value::<CheckResult>(serde_json::json!({
            "check_id": "AC-X-003",
            "verdict": "needs_remediation",
            "really_ran": true,
            "setup_failure": false,
            "real_failure": false,
            "sibling_paths": [],
            "findings": ["baseline pass was vacuous"]
        }))
        .unwrap()
        .remediation_route,
        None
    );
    let line = CheckProgress::new(1).finish_line(
        1,
        "AC-X-001",
        "passed; regression guard: stored judge verdict",
        0.0,
    );
    assert!(line.contains("regression guard: stored judge verdict"));
}

#[test]
fn missing_implementation_failure_is_accepted_without_a_passability_finding() {
    // EXPECTED BEFORE RUN: the check ran and failed on the pre-implementation
    // tree; when the freeze judge attributes that failure to missing
    // implementation, reverify keeps real_failure as evidence but emits no
    // remediation finding and leaves the verdict accepted.
    let mut check = CheckResult {
        check_id: "AC-X-001".into(),
        verdict: "accepted".into(),
        really_ran: true,
        setup_failure: false,
        real_failure: false,
        sibling_paths: Vec::new(),
        findings: Vec::new(),
        not_run_reason: None,
        regression_guard: None,
        remediation_route: None,
    };

    apply_passability_result(&mut check, true, None, None);

    assert_eq!(check.verdict, "accepted");
    assert!(check.real_failure);
    assert!(!check.setup_failure);
    assert!(check.findings.is_empty());
}

#[test]
fn setup_failure_from_freeze_judge_remains_a_finding() {
    // EXPECTED BEFORE RUN: the shared freeze passability judge returns a
    // finding when check setup is refused, so reverify routes it to
    // needs_remediation and preserves the finding text.
    let mut check = CheckResult {
        check_id: "AC-X-001".into(),
        verdict: "accepted".into(),
        really_ran: true,
        setup_failure: false,
        real_failure: false,
        sibling_paths: Vec::new(),
        findings: Vec::new(),
        not_run_reason: None,
        regression_guard: None,
        remediation_route: None,
    };

    apply_passability_result(
        &mut check,
        true,
        Some("cannot pass as written: invalid check setup".into()),
        None,
    );

    assert_eq!(check.verdict, "needs_remediation");
    assert!(check.setup_failure);
    assert!(!check.real_failure);
    assert_eq!(
        check.findings,
        ["cannot pass as written: invalid check setup"]
    );
}

#[test]
fn check_that_passes_on_baseline_has_no_failed_check_finding() {
    // EXPECTED BEFORE RUN: the baseline pass classification follows the
    // freeze probe's passing branch; it is not treated as a failed-check
    // passability finding by reverify.
    let mut check = CheckResult {
        check_id: "AC-X-001".into(),
        verdict: "accepted".into(),
        really_ran: true,
        setup_failure: false,
        real_failure: false,
        sibling_paths: Vec::new(),
        findings: Vec::new(),
        not_run_reason: None,
        regression_guard: None,
        remediation_route: None,
    };

    apply_passability_result(&mut check, false, None, None);

    assert_eq!(check.verdict, "accepted");
    assert!(!check.real_failure);
    assert!(!check.setup_failure);
    assert!(check.findings.is_empty());
}

#[test]
fn baseline_pass_that_cannot_fail_keeps_the_freeze_probe_finding() {
    // EXPECTED BEFORE RUN: when the probe's baseline classification says the
    // check passed and its real mutation proof cannot show a failure, reverify
    // preserves that freeze finding and routes it to remediation.
    let mut check = CheckResult {
        check_id: "AC-X-001".into(),
        verdict: "accepted".into(),
        really_ran: true,
        setup_failure: false,
        real_failure: false,
        sibling_paths: Vec::new(),
        findings: Vec::new(),
        not_run_reason: None,
        regression_guard: None,
        remediation_route: None,
    };

    apply_passability_result(
        &mut check,
        false,
        None,
        Some("check cannot fail on the pre-implementation tree".into()),
    );

    assert_eq!(check.verdict, "needs_remediation");
    assert!(!check.setup_failure);
    assert!(!check.real_failure);
    assert_eq!(
        check.findings,
        ["check cannot fail on the pre-implementation tree"]
    );
    assert_eq!(check.remediation_route, Some(RemediationRoute::Check));
}

#[test]
fn reverify_incomplete_advice_names_the_retry_command() {
    let resume = crate::command::workflow_freeze_budget::FreezeResume::staged("freeze-acceptance");
    let incomplete = crate::command::workflow_freeze_budget::FreezeIncomplete::awaiting_judge(
        &resume.budget,
        &resume.progress,
        vec!["AC-X-001".into()],
        "operational: provider authentication_error: invalid credential",
    );
    let error = anyhow::Error::new(incomplete);
    let text = super::reverify_incomplete_advice(error).to_string();
    assert!(text.contains("archon workflow reverify <run>"), "{text}");
    assert!(
        text.contains("saved observations and verdicts are reused"),
        "{text}"
    );
    assert!(text.contains("authentication_error"), "{text}");
    assert!(!text.contains("Retry the freeze"), "{text}");
}

/// Issue 492: a check rejected from its run with inputs moved aside keeps
/// that run's facts in the reverify record, through its finding, and the
/// record round-trips them; a record written before still loads.
#[test]
fn reverify_rejected_mutation_round_trips_observation() {
    let mut check = CheckResult {
        check_id: "AC-X-001".into(),
        verdict: "accepted".into(),
        really_ran: true,
        setup_failure: false,
        real_failure: false,
        sibling_paths: Vec::new(),
        findings: Vec::new(),
        not_run_reason: None,
        regression_guard: None,
        remediation_route: None,
    };
    let finding = "check 'AC-X-001': it still passed with data/state.txt moved aside; mutation observation: inputs={\"data/state.txt\": \"moved\"}; moved=[\"data/state.txt\"]; mutated_exit_code=Some(0); status/signal: exit status 0; output_excerpt=\"stdout: observed\\nstderr: \"";
    apply_passability_result(&mut check, false, None, Some(finding.to_string()));
    assert_eq!(check.verdict, "needs_remediation");
    assert_eq!(check.remediation_route, Some(RemediationRoute::Check));
    let bytes = serde_json::to_vec(&check).unwrap();
    let back: CheckResult = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(back.findings, [finding]);
    let legacy = serde_json::json!({
        "check_id": "AC-X-001",
        "verdict": "accepted",
        "really_ran": true,
        "setup_failure": false,
        "real_failure": false,
        "sibling_paths": [],
        "findings": []
    });
    let legacy: CheckResult = serde_json::from_value(legacy).unwrap();
    assert!(legacy.findings.is_empty());
}

/// Issue 495: a published check that breaks the site path rule (here a
/// repository-root check reaching its own repository through `..`) is held
/// to the rule again on reverify. It goes to its author as a per-check
/// defect; its sibling is still verified, and reverify does not end.
#[tokio::test]
async fn a_published_site_path_breach_goes_to_its_author_and_siblings_are_verified() {
    use crate::command::workflow_task_set::republish::test_fixture::{frozen_set, write_chain};
    let set = frozen_set(&[
        ("AC-F-001", "test -f input", true),
        ("AC-F-002", "test -f input", true),
    ]);
    let root = set.project.path();
    std::fs::write(root.join("input"), "ready").unwrap();
    let name = root.file_name().unwrap().to_string_lossy().into_owned();
    let mut contract = set.contract();
    contract.acceptance[1].check = AcceptanceCheck::Command {
        command: format!("test -f ../{name}/input"),
        cwd: TrustedCwd::RepoRoot,
    };
    write_chain(
        root,
        &set.tasks,
        &contract,
        archon_workflow::task_set_contract::FreezeGateMode::Observe,
        0,
    );
    crate::command::workflow_task_set::passability_tests_support::commit_project(root);
    let env_vars = archon_core::env_vars::load_env_vars_from(&Default::default());
    let checks = reverify_checks(
        root,
        &set.prd,
        &set.tasks,
        &ArchonConfig::default(),
        &env_vars,
    )
    .await
    .expect("one check's published path never ends reverify");
    let bad = checks
        .checks
        .iter()
        .find(|check| check.check_id == "AC-F-002")
        .expect("the refused check is reported");
    assert_eq!(bad.verdict, "needs_remediation", "{bad:?}");
    assert_eq!(
        bad.remediation_route,
        Some(RemediationRoute::Check),
        "{bad:?}"
    );
    assert!(bad.setup_failure && !bad.really_ran, "{bad:?}");
    assert!(
        bad.findings
            .iter()
            .any(|finding| finding.contains("site path rule") && finding.contains(&name)),
        "{bad:?}"
    );
    assert!(checks.task_subjects.contains(&"AC-F-002".to_string()));
    assert!(!checks.operational_failure);
    let good = checks
        .checks
        .iter()
        .find(|check| check.check_id == "AC-F-001")
        .expect("the sibling is reported");
    assert!(good.really_ran, "{good:?}");
}

/// Issue 544: a value bound only in the acceptance site policy environment
/// -- here the password inside a URL, printed bare, with no credential
/// shape -- is redacted in every place the reverify run stores or prints
/// check output: the guard text and its excerpt, the mutation observation,
/// the stored reverify guard, and the printed progress line.
#[cfg(unix)]
#[tokio::test]
async fn a_site_policy_url_password_never_reaches_any_reverify_sink() {
    use crate::command::workflow_task_set::executability::probe_tests::trees;
    use crate::command::workflow_task_set::republish::test_fixture::write_chain;
    const PASSWORD: &str = "violet-orbit-94";
    let print = r#"v=${RUSTUP_HOME#*user:}; printf '%s\n' "${v%%@*}" >&2"#;
    let guard = format!("test -f tests/focused.rs || {{ {print}; exit 1; }}");
    // It recreates its moved input and passes: a finding that carries the
    // mutation observation.
    let observed = format!(
        r#"{print}; set -- tests/focused.rs; test -f "$1" || printf focused > "$1"; test -f "$1""#
    );
    let trees = trees(&[
        ("AC-LEAK-001", guard.as_str(), TrustedCwd::RepoRoot),
        ("AC-LEAK-002", observed.as_str(), TrustedCwd::RepoRoot),
    ]);
    let project = trees.set.project.path();
    // Precondition: both checks pass the host's check policy, so both run.
    let policy = archon_workflow::task_set_contract::acceptance_policy_findings(&trees.contract());
    assert!(
        policy.is_empty(),
        "{:#?}",
        policy
            .iter()
            .map(|finding| &finding.message)
            .collect::<Vec<_>>()
    );
    write_chain(
        project,
        &trees.set.tasks,
        &trees.contract(),
        archon_workflow::task_set_contract::FreezeGateMode::Observe,
        0,
    );
    std::fs::create_dir_all(project.join(".archon")).unwrap();
    std::fs::write(
        project.join(".archon/config.toml"),
        format!(
            "[workflow.acceptance_execution]\nrepository={:?}\nscratch_parent={:?}\nproject_inputs=[]\nproject_repository_view=\"separate\"\ntoolchain_path=\"/usr/bin:/bin:/usr/sbin:/sbin\"\ntimeout_secs=300\noutput_bytes=8192\nscratch_bytes=16777216\nenvironment={{ RUSTUP_HOME = \"https://user:{PASSWORD}@host/private\" }}\n",
            trees.repo.display().to_string(),
            trees.outside.path().join("scratch").display().to_string(),
        ),
    )
    .unwrap();
    assert!(std::env::var("RUSTUP_HOME").map_or(true, |value| !value.contains(PASSWORD)));
    let env_vars = archon_core::env_vars::load_env_vars_from(&Default::default());
    let checks = reverify_checks(
        project,
        &trees.set.prd,
        &trees.set.tasks,
        &ArchonConfig::default(),
        &env_vars,
    )
    .await
    .expect("reverify runs both checks");
    let record = |id: &str| {
        checks
            .checks
            .iter()
            .find(|check| check.check_id == id)
            .unwrap_or_else(|| panic!("{id} reported: {:#?}", checks.checks))
            .clone()
    };
    let guarded = record("AC-LEAK-001");
    let basis = guarded
        .regression_guard
        .clone()
        .unwrap_or_else(|| panic!("a guard: {guarded:#?}"));
    // Control: the check really printed it (the excerpt and the last line
    // carry its redaction marker).
    // (Capture masks it first, as `[REDACTED]`; the sink redactor would name it.)
    assert!(basis.contains("[REDACTED"), "{basis}");
    let finding = record("AC-LEAK-002").findings.join("\n");
    assert!(finding.contains("mutation observation: "), "{finding}");
    assert!(finding.contains("[REDACTED"), "{finding}");
    for (sink, text) in [
        (
            "guard text and excerpt / stored reverify guard",
            basis.clone(),
        ),
        ("mutation observation", finding),
        ("printed progress", guard_outcome(&basis)),
        (
            "stored record",
            serde_json::to_string(&checks.checks).unwrap(),
        ),
    ] {
        assert!(
            !text.contains(PASSWORD),
            "{sink} leaks the password: {text}"
        );
    }
}

/// Records every judge prompt; accepts every check it is shown.
#[cfg(unix)]
#[derive(Default)]
struct RecordingJudge(std::sync::Mutex<Vec<String>>);

#[cfg(unix)]
#[async_trait::async_trait]
impl archon_workflow::WorkflowLlmClient for RecordingJudge {
    async fn send_message_with_temperature(
        &self,
        messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
        _temperature: f64,
    ) -> archon_workflow::error::WorkflowResult<
        archon_workflow::llm_client_port::WorkflowAgentOutcome,
    > {
        let prompt = serde_json::to_string(&messages).unwrap();
        self.0.lock().unwrap().push(prompt.clone());
        let text = messages[0]["content"].as_str().unwrap_or_default();
        let checks: Vec<serde_json::Value> = text
            .split_once("Checks: ")
            .and_then(|(_, checks)| serde_json::from_str(checks).ok())
            .unwrap_or_default();
        let decisions = checks
            .iter()
            .map(|check| {
                serde_json::json!({
                    "id": check["id"], "verdict": "accepted", "counterexample": "none",
                    "reason": "the absent file is the missing feature"
                })
            })
            .collect::<Vec<_>>();
        Ok(archon_workflow::llm_client_port::WorkflowAgentOutcome {
            content: serde_json::json!({ "decisions": decisions }).to_string(),
            stop_reason: Some("end_turn".into()),
            ..Default::default()
        })
    }

    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> archon_workflow::error::WorkflowResult<
        archon_workflow::llm_client_port::WorkflowAgentOutcome,
    > {
        unreachable!("the judge samples with an explicit temperature")
    }
}

/// Issue 544: a check that fails on the pre-implementation tree goes to the
/// reverify judge with its output; a value bound only in the site policy
/// (a URL password printed bare) never reaches the judge's prompt.
#[cfg(unix)]
#[tokio::test]
async fn a_site_policy_url_password_never_reaches_the_reverify_judge_prompt() {
    use crate::command::workflow_task_set::executability::probe_tests::trees;
    use crate::command::workflow_task_set::republish::test_fixture::write_chain;
    const PASSWORD: &str = "violet-orbit-94";
    let failing = r#"v=${RUSTUP_HOME#*user:}; printf '%s\n' "${v%%@*}" >&2; test -f absent"#;
    let trees = trees(&[("AC-LEAK-003", failing, TrustedCwd::RepoRoot)]);
    let project = trees.set.project.path();
    write_chain(
        project,
        &trees.set.tasks,
        &trees.contract(),
        archon_workflow::task_set_contract::FreezeGateMode::Observe,
        0,
    );
    std::fs::create_dir_all(project.join(".archon")).unwrap();
    std::fs::write(
        project.join(".archon/config.toml"),
        format!(
            "[workflow.acceptance_execution]\nrepository={:?}\nscratch_parent={:?}\nproject_inputs=[]\nproject_repository_view=\"separate\"\ntoolchain_path=\"/usr/bin:/bin:/usr/sbin:/sbin\"\ntimeout_secs=300\noutput_bytes=8192\nscratch_bytes=16777216\nenvironment={{ RUSTUP_HOME = \"https://user:{PASSWORD}@host/private\" }}\n",
            trees.repo.display().to_string(),
            trees.outside.path().join("scratch").display().to_string(),
        ),
    )
    .unwrap();
    let judge = std::sync::Arc::new(RecordingJudge::default());
    let _injected = super::judge_client::inject(project, judge.clone());
    let env_vars = archon_core::env_vars::load_env_vars_from(&Default::default());
    let checks = reverify_checks(
        project,
        &trees.set.prd,
        &trees.set.tasks,
        &ArchonConfig::default(),
        &env_vars,
    )
    .await
    .expect("reverify judges the baseline failure");
    let prompts = judge.0.lock().unwrap().clone();
    assert!(
        !prompts.is_empty(),
        "the judge was asked: {:#?}",
        checks.checks
    );
    let all = prompts.join("\n");
    // Control: the failing run's output is in the prompt, masked.
    assert!(all.contains("REDACTED"), "{all}");
    assert!(
        !all.contains(PASSWORD),
        "the judge prompt leaks the password"
    );
    assert!(
        !serde_json::to_string(&checks.checks)
            .unwrap()
            .contains(PASSWORD)
    );
}
