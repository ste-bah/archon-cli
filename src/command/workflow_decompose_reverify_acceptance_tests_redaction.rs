use super::*;

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
