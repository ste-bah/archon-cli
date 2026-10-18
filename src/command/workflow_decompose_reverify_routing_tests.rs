use super::*;

#[test]
fn remediation_routes_check_defects_and_behavioral_failures_separately() {
    let contract = contract();
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
    let check_defect = CheckResult {
        check_id: "AC-X-001".into(),
        verdict: "needs_remediation".into(),
        really_ran: true,
        setup_failure: false,
        real_failure: false,
        sibling_paths: Vec::new(),
        findings: vec!["passed on base with no can-fail proof".into()],
        not_run_reason: None,
        regression_guard: None,
        remediation_route: Some(RemediationRoute::Check),
    };
    assert_eq!(
        remediation_subjects(std::slice::from_ref(&check_defect), &contract, &skeleton),
        ["AC-X-001"]
    );

    let implementation_failure = CheckResult {
        check_id: "AC-X-001".into(),
        verdict: "needs_remediation".into(),
        really_ran: true,
        setup_failure: false,
        real_failure: true,
        sibling_paths: Vec::new(),
        findings: vec!["implementation does not satisfy criterion".into()],
        not_run_reason: None,
        regression_guard: None,
        remediation_route: Some(RemediationRoute::Task),
    };
    assert_eq!(
        remediation_subjects(
            std::slice::from_ref(&implementation_failure),
            &contract,
            &skeleton
        ),
        ["TASK-X-010"]
    );
    assert_eq!(
        remediation_subjects(
            &[check_defect, implementation_failure],
            &contract,
            &skeleton
        ),
        ["AC-X-001", "TASK-X-010"]
    );
}
