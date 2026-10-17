//! Real fixed-executor coverage for malformed task inputs during reverify.
use super::workflow_decompose_reverify_tests::{archon_binary, fixture};
use archon_workflow::task_set_contract::content_digest;

#[tokio::test]
async fn malformed_body_executor_errors_are_subject_findings_for_each_gate() {
    // EXPECTED BEFORE EXECUTION: the real executor may fail its host-side
    // postcondition parse with TaskBodyInvalid, but reverify must classify
    // that typed error as needs_remediation with a Body finding for
    // TASK-X-010. This applies to body, set-lint, and requirements-trace;
    // unrelated executor errors must remain operational failures.
    let fixture = fixture();
    let task_path = fixture.bodies[0].clone();
    let malformed = "not a task body\n";
    std::fs::write(&task_path, malformed).unwrap();
    struct RestoreEnv(Vec<(&'static str, Option<std::ffi::OsString>)>);
    impl Drop for RestoreEnv {
        fn drop(&mut self) {
            for (name, value) in self.0.drain(..) {
                match value {
                    Some(value) => unsafe { std::env::set_var(name, value) },
                    None => unsafe { std::env::remove_var(name) },
                }
            }
        }
    }
    let restored = ["HOME", "XDG_DATA_HOME"]
        .into_iter()
        .map(|name| (name, std::env::var_os(name)))
        .collect();
    unsafe {
        std::env::set_var("HOME", &fixture.project);
        std::env::set_var("XDG_DATA_HOME", fixture.project.join(".test-data"));
    }
    let _restore_env = RestoreEnv(restored);
    let binary = archon_binary();
    let revision = env!("ARCHON_GIT_HASH");
    let catalog =
        super::super::workflow_host_command_catalog::fixed_decomposition_catalog(revision).unwrap();
    let prd_path = fixture.project.join("prds/PRD-X.md");
    let task_root = fixture.project.join("tasks/PRD-X");
    let base_context = super::super::workflow_host_command_catalog::HostCommandResolutionContext {
        program: binary,
        project_root: fixture.project.clone(),
        prd_path: prd_path.clone(),
        prd_digest: content_digest(&std::fs::read(&prd_path).unwrap()),
        task_root: task_root.clone(),
        run_staging_root: fixture
            .store
            .run_dir(&fixture.run_id)
            .join("host-command-staging"),
        frozen_task_id: None,
        frozen_task_file: None,
        freeze_provider_environment: Default::default(),
        acceptance_environment_allowlist: Vec::new(),
        gate_mode: archon_core::config::GateMode::Enforce,
    };
    let run_root = fixture.store.run_dir(&fixture.run_id);
    let staging_root = run_root.join("host-command-staging");
    assert!(!staging_root.starts_with(&task_root));
    assert!(!task_root.starts_with(&staging_root));
    let generation = fixture
        .store
        .load_state(&fixture.run_id)
        .unwrap()
        .generation;
    println!(
        "EVIDENCE malformed_body_gates BEFORE: subject=TASK-X-010 bytes={} capabilities=land-task-body,task-set-lint,requirements-trace expected=needs_remediation scope=Body provider_calls=none",
        malformed.len()
    );
    for command_id in ["land-task-body", "task-set-lint", "requirements-trace"] {
        let mut context = base_context.clone();
        let stdin = if command_id == "land-task-body" {
            context.frozen_task_id = Some("TASK-X-010".into());
            context.frozen_task_file = Some(task_path.clone());
            Some(malformed.to_string())
        } else {
            None
        };
        let request = archon_workflow::HostCommandRequest::new(command_id, stdin).unwrap();
        let executor = super::super::workflow_host_command_exec::FixedHostCommandExecutor::new(
            catalog.clone(),
            context,
            run_root.clone(),
        );
        let result = executor
            .execute_observed_only(request, Some(generation))
            .await
            .unwrap_or_else(|error| {
                let gate_id = match command_id {
                    "land-task-body" => crate::command::workflow_gate::GateId::WorkflowLintTaskFile,
                    "task-set-lint" => crate::command::workflow_gate::GateId::WorkflowLintTaskSet,
                    _ => crate::command::workflow_gate::GateId::RequirementsTrace,
                };
                super::workflow_decompose_reverify_assessment::executor_error_result(
                    error,
                    gate_id,
                    "TASK-X-010",
                )
            });
        let (verdict, findings) = super::workflow_decompose_reverify::gate_assessment(&result);
        assert_eq!(verdict, "needs_remediation", "{command_id}: {findings:?}");
        assert_eq!(findings.len(), 1, "{command_id}: {findings:?}");
        assert!(
            findings[0].contains("TASK-X-010"),
            "{command_id}: {findings:?}"
        );
        assert!(
            findings[0].contains("found no fenced"),
            "{command_id}: {findings:?}"
        );
        let finding = result
            .gate_envelope
            .as_ref()
            .unwrap()
            .policy_findings
            .first()
            .unwrap();
        assert_eq!(
            finding.remediation_scope,
            archon_workflow::RemediationScope::Body
        );
        assert_eq!(finding.subject, "TASK-X-010");
        if staging_root.exists() {
            for entry in std::fs::read_dir(&staging_root).unwrap() {
                let call_staging = entry.unwrap().path();
                assert!(call_staging.is_dir());
                assert!(
                    !call_staging.join("TASK-X-010.md").exists(),
                    "staged candidate remained at {}",
                    call_staging.display()
                );
            }
        }
        println!(
            "EVIDENCE malformed_body_gates AFTER: capability={} verdict={} subject={} scope={:?} parse_error_present=true",
            command_id, verdict, finding.subject, finding.remediation_scope
        );
    }
    // The reverify path uses judge_only, which stages --task-file outside the
    // task root. The call directory may remain for the anchor, but its body
    // copy must be gone once the executor has completed.
    let mut context = base_context;
    context.frozen_task_id = Some("TASK-X-010".into());
    context.frozen_task_file = Some(task_path);
    let request =
        archon_workflow::HostCommandRequest::new("land-task-body", Some(malformed.to_string()))
            .unwrap();
    let executor = super::super::workflow_host_command_exec::FixedHostCommandExecutor::new(
        catalog, context, run_root,
    );
    println!(
        "EVIDENCE reverify_staging BEFORE: task_root={} staging_root={} body_copy_expected_outside=true",
        task_root.display(),
        staging_root.display()
    );
    let result = executor.execute_judge_only(request, Some(generation)).await;
    assert!(
        matches!(
            result,
            Err(archon_workflow::WorkflowError::TaskBodyInvalid(_))
        ),
        "malformed body should surface as a subject parse finding: {result:?}"
    );
    if staging_root.exists() {
        for entry in std::fs::read_dir(&staging_root).unwrap() {
            let call_staging = entry.unwrap().path();
            assert!(call_staging.is_dir());
            assert!(!call_staging.join("TASK-X-010.md").exists());
        }
    }
    println!(
        "EVIDENCE reverify_staging AFTER: staging_outside_task_root=true staged_body_removed=true"
    );
}

#[test]
fn unrelated_executor_errors_remain_operational_during_reverify() {
    // EXPECTED BEFORE CLASSIFICATION: host I/O and setup errors are not task
    // body parse failures, so they remain operational and retain their cause.
    let error = archon_workflow::WorkflowError::HostOperational("staging unavailable".into());
    let result = super::workflow_decompose_reverify_assessment::executor_error_result(
        error,
        crate::command::workflow_gate::GateId::WorkflowLintTaskFile,
        "TASK-X-010",
    );
    println!(
        "EVIDENCE unrelated_executor_error BEFORE: kind=HostOperational cause=staging unavailable"
    );
    let (verdict, findings) = super::workflow_decompose_reverify::gate_assessment(&result);
    assert_eq!(verdict, "operational_failure");
    assert_eq!(findings.len(), 1);
    assert!(findings[0].contains("staging unavailable"));
    assert!(findings[0].contains("no policy verdict"));
    println!(
        "EVIDENCE unrelated_executor_error AFTER: verdict={verdict} operational_cause_preserved=true"
    );
}
