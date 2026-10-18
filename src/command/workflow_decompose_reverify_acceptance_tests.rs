use super::*;
use archon_workflow::task_set_contract::{
    AcceptanceCheck, AcceptanceCriterion, JudgeVerdict, TrustedCwd,
};
use archon_workflow::task_skeleton::{FrozenTask, TaskSkeleton};

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

#[tokio::test]
async fn reverify_report_sibling_paths_come_from_each_real_probe_check() {
    use crate::command::workflow_task_set::executability::probe_tests::trees;
    use archon_workflow::task_set_contract::TrustedCwd;

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
    // needs_remediation produces its owning task.
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
    };
    let not_run = not_run_checks(&frozen_check_plan(&contract));
    assert!(
        remediation_task_subjects(
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
    };
    assert_eq!(
        remediation_task_subjects(&[setup_failure], &contract, &skeleton),
        ["TASK-X-010"]
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
