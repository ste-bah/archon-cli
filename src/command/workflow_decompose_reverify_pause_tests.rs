//! Pause authorization and ordinary executor regressions.
use super::workflow_decompose_reverify_tests;
use archon_workflow::task_set_contract::content_digest;

fn paused_fixture_with_unconsumed_report() -> workflow_decompose_reverify_tests::Fixture {
    let fixture = workflow_decompose_reverify_tests::fixture();
    let original = std::fs::read(&fixture.bodies[0]).unwrap();
    std::fs::write(&fixture.bodies[0], b"not a task body\n").unwrap();
    let first = workflow_decompose_reverify_tests::invoke_reverify(
        &workflow_decompose_reverify_tests::archon_binary(),
        &fixture,
    );
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert_eq!(
        fixture.store.load_state(&fixture.run_id).unwrap().status,
        archon_workflow::RunStatus::Paused
    );
    std::fs::write(&fixture.bodies[0], original).unwrap();
    fixture
}

#[tokio::test]
async fn pause_after_first_reverify_record_stops_further_persistence() {
    let fixture = paused_fixture_with_unconsumed_report();
    let v2 = archon_workflow::WorkflowV2ResultStore::new(
        fixture.store.run_dir(&fixture.run_id).join("v2"),
    );
    let before_ids = v2
        .load_call_records()
        .unwrap()
        .into_iter()
        .map(|record| record.call.id)
        .collect::<std::collections::BTreeSet<_>>();
    let store = fixture.store.clone();
    let run_id = fixture.run_id.clone();
    crate::command::workflow::workflow_decompose_reverify::persist::set_after_persist_hook(
        move || {
            archon_workflow::LifecycleController::new(store)
                .apply(&run_id, archon_workflow::LifecycleAction::Pause)
                .unwrap();
            false
        },
    );
    let stopped = crate::command::workflow::workflow_decompose_reverify::reverify(
        &fixture.project,
        &fixture.run_id,
        &archon_core::config::ArchonConfig::default(),
        &archon_core::env_vars::load_env_vars_from(&std::collections::HashMap::new()),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(
            stopped.downcast_ref::<archon_workflow::WorkflowError>(),
            Some(archon_workflow::WorkflowError::ControlPaused(_))
        ),
        "expected pause control outcome, got {stopped:#}"
    );
    let records = v2.load_call_records().unwrap();
    let (events, _) =
        archon_workflow::events::read_event_log(&fixture.store.events_path(&fixture.run_id))
            .unwrap();
    let invocation = events
        .iter()
        .rev()
        .find(|event| {
            event.kind == archon_workflow::WorkflowEventKind::DecompositionReverifyStarted
        })
        .unwrap()
        .detail["invocation_id"]
        .as_str()
        .unwrap();
    let newly_persisted = records
        .iter()
        .filter(|record| !before_ids.contains(&record.call.id))
        .count();
    assert_eq!(
        newly_persisted, 1,
        "unexpected records after stop; invocation={invocation}"
    );
    assert_eq!(
        fixture.store.load_state(&fixture.run_id).unwrap().status,
        archon_workflow::RunStatus::Paused
    );
}

#[tokio::test]
async fn final_reverify_publication_yields_to_operator_pause_and_cancel() {
    for (action, expected) in [
        (
            archon_workflow::LifecycleAction::Pause,
            archon_workflow::RunStatus::Paused,
        ),
        (
            archon_workflow::LifecycleAction::Cancel,
            archon_workflow::RunStatus::Cancelled,
        ),
    ] {
        let fixture = paused_fixture_with_unconsumed_report();
        let prior_report_files = std::fs::read_dir(
            fixture
                .store
                .run_dir(&fixture.run_id)
                .join("decomposition/reverify/reports"),
        )
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<std::collections::BTreeSet<_>>();
        let prior_report_ids = archon_workflow::WorkflowV2ResultStore::new(
            fixture.store.run_dir(&fixture.run_id).join("v2"),
        )
        .load_call_records()
        .unwrap()
        .into_iter()
        .filter(|record| record.call.method == archon_workflow::WorkflowV2HostMethod::FinalReport)
        .map(|record| record.call.id)
        .collect::<std::collections::BTreeSet<_>>();
        let store = fixture.store.clone();
        let run_id = fixture.run_id.clone();
        crate::command::workflow::workflow_decompose_reverify::publish::set_before_final_publication_hook(move || {
                archon_workflow::LifecycleController::new(store)
                    .apply(&run_id, action)
                    .unwrap();
            });

        let error = crate::command::workflow::workflow_decompose_reverify::reverify(
            &fixture.project,
            &fixture.run_id,
            &archon_core::config::ArchonConfig::default(),
            &archon_core::env_vars::load_env_vars_from(&std::collections::HashMap::new()),
        )
        .await
        .unwrap_err();
        let control = error
            .downcast_ref::<archon_workflow::WorkflowError>()
            .unwrap_or_else(|| panic!("expected a control outcome, got {error:#}"));
        match expected {
            archon_workflow::RunStatus::Paused => {
                assert!(matches!(
                    control,
                    archon_workflow::WorkflowError::ControlPaused(_)
                ));
            }
            archon_workflow::RunStatus::Cancelled => {
                assert!(matches!(
                    control,
                    archon_workflow::WorkflowError::ControlCancelled(_)
                ));
            }
            _ => unreachable!(),
        }
        let after = fixture.store.load_state(&fixture.run_id).unwrap();
        assert_eq!(after.status, expected);
        let run_dir = fixture.store.run_dir(&fixture.run_id);
        let (events, damaged) =
            archon_workflow::events::read_event_log(&fixture.store.events_path(&fixture.run_id))
                .unwrap();
        assert_eq!(damaged, 0);
        let started = events
            .iter()
            .rev()
            .find(|event| {
                event.kind == archon_workflow::WorkflowEventKind::DecompositionReverifyStarted
            })
            .unwrap();
        let invocation_id = started
            .detail
            .get("invocation_id")
            .and_then(serde_json::Value::as_str)
            .unwrap();
        let report_id = format!("{invocation_id}-report");
        let v2 = archon_workflow::WorkflowV2ResultStore::new(run_dir.join("v2"));
        let report_files = std::fs::read_dir(run_dir.join("decomposition/reverify/reports"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(report_files, prior_report_files);
        let record_ids = v2
            .load_call_records()
            .unwrap()
            .into_iter()
            .filter(|record| {
                record.call.method == archon_workflow::WorkflowV2HostMethod::FinalReport
            })
            .map(|record| record.call.id)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(record_ids, prior_report_ids);
        assert!(!v2.result_path(&report_id).exists());
        assert!(!events.iter().any(|event| event.seq > started.seq
            && event.kind == archon_workflow::WorkflowEventKind::DecompositionReverified));
        let retry = workflow_decompose_reverify_tests::invoke_reverify(
            &workflow_decompose_reverify_tests::archon_binary(),
            &fixture,
        );
        assert!(!retry.status.success());
        let retry_stderr = String::from_utf8_lossy(&retry.stderr);
        assert!(
            retry_stderr.contains("remediation already started")
                || retry_stderr.contains("status Cancelled"),
            "unexpected control-event admission after {expected:?}: {retry_stderr}"
        );
    }
}

pub(super) fn verify_second_reverify_gates(
    binary: &std::path::Path,
    fixture: &workflow_decompose_reverify_tests::Fixture,
) {
    let second = workflow_decompose_reverify_tests::invoke_reverify(binary, fixture);
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    let latest = super::super::workflow_decompose_latest_report::latest_report(
        &fixture.store,
        &fixture.run_id,
    )
    .unwrap()
    .unwrap();
    let report: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            fixture
                .store
                .run_dir(&fixture.run_id)
                .join(format!("decomposition/reverify/reports/{}.json", latest.id)),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(report["outcome"], "needs_remediation");
    for gate in report["set_gates"]
        .as_array()
        .unwrap()
        .iter()
        .chain(report["frozen_gates"].as_array().unwrap())
    {
        assert!(
            gate["verdict"] == "accepted" || gate["verdict"] == "needs_remediation",
            "{gate:#}"
        );
    }
    println!(
        "EVIDENCE paused_reverify_gates AFTER: prior_outcome=needs_remediation paused=true ledger_unconsumed=true second_outcome={} set_gates={} frozen_gates={} policy_verdicts=true",
        report["outcome"],
        report["set_gates"].as_array().unwrap().len(),
        report["frozen_gates"].as_array().unwrap().len()
    );
}
#[tokio::test]
async fn ordinary_host_executor_still_refuses_reverify_paused_run() {
    let fixture = workflow_decompose_reverify_tests::fixture();
    std::fs::write(&fixture.bodies[0], "not a task body\n").unwrap();
    let output = workflow_decompose_reverify_tests::invoke_reverify(
        &workflow_decompose_reverify_tests::archon_binary(),
        &fixture,
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let run = fixture.store.load_state(&fixture.run_id).unwrap();
    assert_eq!(run.status, archon_workflow::RunStatus::Paused);
    let revision = env!("ARCHON_GIT_HASH");
    let catalog =
        super::super::workflow_host_command_catalog::fixed_decomposition_catalog(revision).unwrap();
    let prd_path = fixture.project.join("prds/PRD-X.md");
    let task_root = fixture.project.join("tasks/PRD-X");
    let context = super::super::workflow_host_command_catalog::HostCommandResolutionContext {
        program: workflow_decompose_reverify_tests::archon_binary(),
        project_root: fixture.project.clone(),
        prd_path: prd_path.clone(),
        prd_digest: content_digest(&std::fs::read(&prd_path).unwrap()),
        task_root,
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
    let executor = super::super::workflow_host_command_exec::FixedHostCommandExecutor::new(
        catalog,
        context,
        fixture.store.run_dir(&fixture.run_id),
    );
    let error = executor
        .execute_observed_only(
            archon_workflow::HostCommandRequest::new("task-set-lint", None).unwrap(),
            Some(run.generation),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(error, archon_workflow::WorkflowError::ControlPaused(_)),
        "{error:?}"
    );
}

#[tokio::test]
async fn paused_run_authorization_is_invocation_and_generation_bound() {
    let fixture = workflow_decompose_reverify_tests::fixture();
    std::fs::write(&fixture.bodies[0], "not a task body\n").unwrap();
    let binary = workflow_decompose_reverify_tests::archon_binary();
    let first = workflow_decompose_reverify_tests::invoke_reverify(&binary, &fixture);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let lease = crate::command::workflow_executor_lease::acquire(
        &fixture.store.run_dir(&fixture.run_id),
        &fixture.run_id,
    )
    .unwrap();
    let generation = fixture
        .store
        .load_state(&fixture.run_id)
        .unwrap()
        .generation;
    let invocation_id = format!("test-reverify-{}", uuid::Uuid::new_v4());
    archon_workflow::WorkflowEventLog::new(fixture.store.clone())
        .emit(
            &fixture.run_id,
            fixture.store.next_event_seq(&fixture.run_id).unwrap(),
            archon_workflow::WorkflowEventKind::DecompositionReverifyStarted,
            serde_json::json!({"invocation_id": invocation_id}),
        )
        .unwrap();
    let authorization =
        crate::command::workflow_host_command_operational::PausedRunAuthorization::new(
            invocation_id.clone(),
            generation,
            &lease,
        );
    assert!(
        crate::command::workflow_host_command_operational::require_run_owned_authorized(
            &fixture.store,
            &fixture.run_id,
            generation,
            Some(&authorization),
        )
        .is_ok()
    );
    let different = crate::command::workflow_host_command_operational::PausedRunAuthorization::new(
        "different-invocation".into(),
        generation,
        &lease,
    );
    assert!(matches!(
        crate::command::workflow_host_command_operational::require_run_owned_authorized(
            &fixture.store,
            &fixture.run_id,
            generation,
            Some(&different),
        ),
        Err(archon_workflow::WorkflowError::ControlPaused(_))
    ));
    assert!(matches!(
        crate::command::workflow_host_command_operational::require_run_owned_authorized(
            &fixture.store,
            &fixture.run_id,
            generation + 1,
            Some(&authorization),
        ),
        Err(archon_workflow::WorkflowError::ControlPaused(_)
            | archon_workflow::WorkflowError::ControlCancelled(_))
    ));
    drop(lease);
    assert!(matches!(
        crate::command::workflow_host_command_operational::require_run_owned_authorized(
            &fixture.store,
            &fixture.run_id,
            generation,
            Some(&authorization),
        ),
        Err(archon_workflow::WorkflowError::ControlPaused(_))
    ));
    let pause = std::process::Command::new(&binary)
        .args(["workflow", "pause", &fixture.run_id])
        .current_dir(&fixture.project)
        .env("HOME", &fixture.project)
        .env("XDG_DATA_HOME", fixture.project.join(".test-data"))
        .output()
        .unwrap();
    assert!(
        pause.status.success(),
        "{}",
        String::from_utf8_lossy(&pause.stderr)
    );
    assert!(matches!(
        crate::command::workflow_host_command_operational::require_run_owned_authorized(
            &fixture.store,
            &fixture.run_id,
            generation,
            Some(&authorization),
        ),
        Err(archon_workflow::WorkflowError::ControlPaused(_)
            | archon_workflow::WorkflowError::ControlCancelled(_))
    ));
    let after_operator_pause = fixture.store.load_state(&fixture.run_id).unwrap();
    assert!(matches!(
        crate::command::workflow_host_command_operational::supervisor_signal_authorized(
            &fixture.store,
            &after_operator_pause,
            generation,
            Some(&authorization),
        ),
        Some(
            crate::command::workflow_host_command_supervisor::HostCommandSignal::Paused
                | crate::command::workflow_host_command_supervisor::HostCommandSignal::Cancelled
        )
    ));
    let refused = workflow_decompose_reverify_tests::invoke_reverify(&binary, &fixture);
    assert!(!refused.status.success());

    let operator_fixture = workflow_decompose_reverify_tests::fixture();
    std::fs::write(&operator_fixture.bodies[0], "not a task body\n").unwrap();
    let first = workflow_decompose_reverify_tests::invoke_reverify(&binary, &operator_fixture);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let operator_pause = std::process::Command::new(&binary)
        .args(["workflow", "pause", &operator_fixture.run_id])
        .current_dir(&operator_fixture.project)
        .env("HOME", &operator_fixture.project)
        .env("XDG_DATA_HOME", operator_fixture.project.join(".test-data"))
        .output()
        .unwrap();
    assert!(
        operator_pause.status.success(),
        "{}",
        String::from_utf8_lossy(&operator_pause.stderr)
    );
    let refused = workflow_decompose_reverify_tests::invoke_reverify(&binary, &operator_fixture);
    assert!(!refused.status.success());
    let refusal = String::from_utf8_lossy(&refused.stderr);
    assert!(
        refusal.contains("not eligible") || refusal.contains("remediation already started"),
        "{refusal}"
    );
}
