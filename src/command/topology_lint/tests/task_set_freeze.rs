use super::*;

#[test]
fn directory_lint_renders_kind_mismatch_as_information_only() {
    use archon_workflow::task_set_contract::{TASK_SKELETON_FILE, TASK_SKELETON_LOCK_FILE};

    let temp = tempfile::tempdir().unwrap();
    let task_path = write_task_file_lint_fixture(temp.path());
    let tasks = task_path.parent().unwrap();
    let skeleton_path = tasks.join(TASK_SKELETON_FILE);
    let mut skeleton: archon_workflow::task_skeleton::TaskSkeleton =
        serde_json::from_slice(&std::fs::read(&skeleton_path).unwrap()).unwrap();
    skeleton.tasks.insert(
        0,
        archon_workflow::task_skeleton::FrozenTask {
            task_id: "TASK-X-001".into(),
            file_name: "TASK-X-001-base.md".into(),
            depends_on: Vec::new(),
            blocks: Vec::new(),
            implements: Vec::new(),
            deliverable_contracts: vec![
                archon_workflow::task_universe::WorkflowV2DeliverableContract {
                    kind: "producer_label".into(),
                    artifact_path: "out.json".into(),
                    typed_verifier_command: Some("grep -q required {artifact_path}".into()),
                    ..Default::default()
                },
            ],
        },
    );
    skeleton.tasks[1].depends_on = vec![archon_workflow::task_skeleton::FrozenDependency {
        task_id: "TASK-X-001".into(),
        consumes: vec![archon_workflow::task_skeleton::ConsumedArtifact {
            artifact_path: "out.json".into(),
            kind: Some("consumer_label".into()),
            ..Default::default()
        }],
        ordering_only: false,
    }];
    let bytes = serde_json::to_vec_pretty(&skeleton).unwrap();
    std::fs::write(&skeleton_path, &bytes).unwrap();
    let digest = archon_workflow::task_set_contract::content_digest(&bytes);
    std::fs::write(
        tasks.join(TASK_SKELETON_LOCK_FILE),
        serde_json::to_vec_pretty(&archon_workflow::task_skeleton::TaskSkeletonLock {
            algorithm: "blake3".into(),
            digest: digest.clone(),
            acceptance_digest: skeleton.acceptance_digest.clone(),
            gate: clean_stamp(),
        })
        .unwrap(),
    )
    .unwrap();
    let pin_path = crate::command::workflow_task_set::acceptance_pin_path(temp.path(), tasks);
    let mut pin: archon_workflow::task_set_contract::AcceptancePin =
        serde_json::from_slice(&std::fs::read(&pin_path).unwrap()).unwrap();
    pin.skeleton_digest = Some(digest);
    pin.skeleton_gate = Some(clean_stamp());
    std::fs::write(pin_path, serde_json::to_vec_pretty(&pin).unwrap()).unwrap();

    let report = run_lint(temp.path(), &LintSource::Tasks(tasks.to_path_buf())).unwrap();
    assert!(
        report.contains("dependency contract information (non-blocking)"),
        "{report}"
    );
    assert!(report.contains("informational kind mismatch"), "{report}");
    let blockers = blocking_findings(temp.path(), &LintSource::Tasks(tasks.to_path_buf()));
    assert!(
        !blockers
            .iter()
            .any(|finding| finding.contains("kind mismatch")),
        "{blockers:?}"
    );
}

#[test]
fn off_mode_skips_lint_input_and_observe_records_real_policy_findings() {
    let off = tempfile::tempdir().unwrap();
    let source = LintSource::TaskFile(off.path().join("does-not-exist.md"));
    let disposition = crate::command::workflow_gate::run_sync_gate(
        off.path(),
        archon_core::config::GateMode::Off,
        crate::command::workflow_gate::GateId::WorkflowLintTaskFile,
        || evaluate_lint(off.path(), &source, archon_core::config::GateMode::Off),
    )
    .unwrap();
    assert_eq!(
        disposition.report(),
        crate::command::workflow_gate::OFF_MESSAGE
    );
    assert!(!off.path().join(".archon").exists());

    let observed = tempfile::tempdir().unwrap();
    let task = write_task_file_lint_fixture(observed.path());
    let raw = std::fs::read_to_string(&task).unwrap();
    std::fs::write(&task, raw.replace("status: ready", "status: blocked")).unwrap();
    let source = LintSource::TaskFile(task);
    let disposition = crate::command::workflow_gate::run_sync_gate(
        observed.path(),
        archon_core::config::GateMode::Observe,
        crate::command::workflow_gate::GateId::WorkflowLintTaskFile,
        || {
            evaluate_lint(
                observed.path(),
                &source,
                archon_core::config::GateMode::Observe,
            )
        },
    )
    .unwrap();
    assert!(!disposition.is_blocked());
    assert!(
        disposition
            .diagnostics()
            .iter()
            .any(|line| line.contains("status: blocked"))
    );
    assert!(crate::command::workflow_gate::shadow_log_path(observed.path()).is_file());
}

#[test]
fn enforce_lint_refuses_predecessor_freeze_with_observed_findings() {
    let temp = tempfile::tempdir().unwrap();
    let task = write_task_file_lint_fixture(temp.path());
    let tasks = task.parent().unwrap();
    let pin_path = crate::command::workflow_task_set::acceptance_pin_path(temp.path(), tasks);
    let mut pin: archon_workflow::task_set_contract::AcceptancePin =
        serde_json::from_slice(&std::fs::read(&pin_path).unwrap()).unwrap();
    pin.acceptance_gate.mode = archon_workflow::task_set_contract::FreezeGateMode::Observe;
    pin.acceptance_gate.finding_count = 1;
    pin.acceptance_gate.findings_digest =
        archon_workflow::task_set_contract::content_digest(b"observed finding");
    let lock_path = tasks.join(archon_workflow::task_set_contract::ACCEPTANCE_LOCK_FILE);
    let mut lock: archon_workflow::task_set_contract::AcceptanceLock =
        serde_json::from_slice(&std::fs::read(&lock_path).unwrap()).unwrap();
    lock.gate = pin.acceptance_gate.clone();
    std::fs::write(&pin_path, serde_json::to_vec_pretty(&pin).unwrap()).unwrap();
    std::fs::write(&lock_path, serde_json::to_vec_pretty(&lock).unwrap()).unwrap();

    let evaluation = evaluate_lint(
        temp.path(),
        &LintSource::TaskFile(task),
        archon_core::config::GateMode::Enforce,
    )
    .unwrap();
    assert!(
        evaluation.findings.iter().any(|finding| {
            finding.text.contains("predecessor acceptance freeze")
                && finding.text.contains("re-freeze under enforce")
        }),
        "{:?}",
        evaluation.findings
    );
}

#[test]
fn partial_acceptance_freeze_is_operational_even_in_observe_mode() {
    use archon_workflow::task_set_contract::{
        ACCEPTANCE_LOCK_FILE, TASK_SKELETON_FILE, TASK_SKELETON_LOCK_FILE,
    };

    let temp = tempfile::tempdir().unwrap();
    let task = write_task_file_lint_fixture(temp.path());
    let tasks = task.parent().unwrap();
    let pin_path = crate::command::workflow_task_set::acceptance_pin_path(temp.path(), tasks);
    std::fs::remove_file(pin_path).unwrap();
    std::fs::remove_file(tasks.join(ACCEPTANCE_LOCK_FILE)).unwrap();
    std::fs::remove_file(tasks.join(TASK_SKELETON_FILE)).unwrap();
    std::fs::remove_file(tasks.join(TASK_SKELETON_LOCK_FILE)).unwrap();

    let source = LintSource::Tasks(tasks.to_path_buf());
    let error = crate::command::workflow_gate::run_sync_gate(
        temp.path(),
        archon_core::config::GateMode::Observe,
        crate::command::workflow_gate::GateId::WorkflowLintTaskSet,
        || evaluate_lint(temp.path(), &source, archon_core::config::GateMode::Observe),
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("partial acceptance freeze"), "{error}");
    assert!(!crate::command::workflow_gate::shadow_log_path(temp.path()).exists());
}

#[test]
fn graph_lowering_failure_preserves_report_but_is_operational_in_observe() {
    let temp = tempfile::tempdir().unwrap();
    let tasks = temp.path().join("tasks/PRD-X");
    std::fs::create_dir_all(&tasks).unwrap();
    for (task_id, dependency) in [("TASK-X-010", "TASK-X-020"), ("TASK-X-020", "TASK-X-010")] {
        std::fs::write(
            tasks.join(format!("{task_id}-body.md")),
            format!(
                "# Body\n\n```yaml\ntask_id: {task_id}\ntitle: Body\ncomplexity: medium\nstatus: ready\ndepends_on: [{dependency}]\nblocks: []\nimplements: []\nrequired_env_keys: []\nrequired_tools: [sh]\ndeliverable_contracts: []\n```\n\n## Focused Tests\n- `sh -c 'exit 1'`\n"
            ),
        )
        .unwrap();
    }
    let source = LintSource::Tasks(tasks);
    let disposition = crate::command::workflow_gate::run_sync_gate(
        temp.path(),
        archon_core::config::GateMode::Observe,
        crate::command::workflow_gate::GateId::WorkflowLintTaskSet,
        || evaluate_lint(temp.path(), &source, archon_core::config::GateMode::Observe),
    )
    .unwrap();

    assert!(disposition.report().contains("NOT ANALYSED"));
    let error = disposition.require_allowed().unwrap_err().to_string();
    assert!(error.contains("dependency cycle"), "{error}");
    assert!(!crate::command::workflow_gate::shadow_log_path(temp.path()).exists());
}

#[test]
fn directory_lint_rejects_stale_prd_content_even_when_ids_are_unchanged() {
    let temp = tempfile::tempdir().unwrap();
    let task = write_task_file_lint_fixture(temp.path());
    let tasks = task.parent().unwrap().to_path_buf();
    let prd = temp.path().join("prds/PRD-X.md");
    let raw = std::fs::read_to_string(&prd).unwrap();
    std::fs::write(
        &prd,
        raw.replace("output is valid", "output is valid after mutation"),
    )
    .unwrap();

    let error = evaluate_lint(
        temp.path(),
        &LintSource::Tasks(tasks),
        archon_core::config::GateMode::Observe,
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("PRD digest mismatch"), "{error}");
    assert!(
        error.contains("re-run workflow freeze-acceptance"),
        "{error}"
    );
}
