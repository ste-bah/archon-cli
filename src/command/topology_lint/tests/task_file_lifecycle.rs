use super::*;

fn write_unfrozen_task(root: &Path) -> std::path::PathBuf {
    let tasks = root.join("tasks/PRD-X");
    std::fs::create_dir_all(&tasks).unwrap();
    let task = tasks.join("TASK-X-010-body.md");
    std::fs::write(
        &task,
        "# Body\n\n```yaml\ntask_id: TASK-X-010\ntitle: Body\ncomplexity: medium\nstatus: ready\ndepends_on: []\nblocks: []\nimplements: []\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n```\n\n## Focused Tests\n- Verification remains to be made runnable.\n",
    )
    .unwrap();
    task
}

fn lint_sources(task: &Path) -> [(LintSource, crate::command::workflow_gate::GateId); 2] {
    [
        (
            LintSource::TaskFile(task.to_path_buf()),
            crate::command::workflow_gate::GateId::WorkflowLintTaskFile,
        ),
        (
            LintSource::Tasks(task.parent().unwrap().to_path_buf()),
            crate::command::workflow_gate::GateId::WorkflowLintTaskSet,
        ),
    ]
}

#[test]
fn unfrozen_task_file_and_task_set_evaluate_in_observe_and_enforce() {
    let temp = tempfile::tempdir().unwrap();
    let task = write_unfrozen_task(temp.path());

    for mode in [
        archon_core::config::GateMode::Observe,
        archon_core::config::GateMode::Enforce,
    ] {
        for (source, gate_id) in lint_sources(&task) {
            let disposition =
                crate::command::workflow_gate::run_sync_gate(temp.path(), mode, gate_id, || {
                    evaluate_lint(temp.path(), &source, mode)
                })
                .expect("a wholly absent freeze chain is valid legacy input");

            assert!(
                disposition.report().contains("legacy compatibility"),
                "{}",
                disposition.report()
            );
            match mode {
                archon_core::config::GateMode::Observe => {
                    disposition
                        .require_allowed()
                        .expect("observe admits evaluated policy findings");
                    assert!(
                        disposition
                            .diagnostics()
                            .iter()
                            .any(|line| line.contains("[shadow]") && line.contains("runnable")),
                        "{:?}",
                        disposition.diagnostics()
                    );
                }
                archon_core::config::GateMode::Enforce => {
                    let error = disposition.require_allowed().unwrap_err().to_string();
                    assert!(error.contains("runnable"), "{error}");
                    assert!(!error.contains("acceptance pin"), "{error}");
                    assert!(!error.contains("freeze-acceptance"), "{error}");
                }
                archon_core::config::GateMode::Off => unreachable!(),
            }
        }
    }

    assert!(crate::command::workflow_gate::shadow_log_path(temp.path()).is_file());
}

#[test]
fn task_file_lint_accepts_the_complete_acceptance_only_phase() {
    use archon_workflow::task_set_contract::{TASK_SKELETON_FILE, TASK_SKELETON_LOCK_FILE};

    let temp = tempfile::tempdir().unwrap();
    let task = write_task_file_lint_fixture(temp.path());
    let tasks = task.parent().unwrap();
    std::fs::remove_file(tasks.join(TASK_SKELETON_FILE)).unwrap();
    std::fs::remove_file(tasks.join(TASK_SKELETON_LOCK_FILE)).unwrap();
    let pin_path = crate::command::workflow_task_set::acceptance_pin_path(temp.path(), tasks);
    let mut pin: archon_workflow::task_set_contract::AcceptancePin =
        serde_json::from_slice(&std::fs::read(&pin_path).unwrap()).unwrap();
    pin.skeleton_digest = None;
    pin.skeleton_gate = None;
    std::fs::write(&pin_path, serde_json::to_vec_pretty(&pin).unwrap()).unwrap();

    let source = LintSource::TaskFile(task);
    let disposition = crate::command::workflow_gate::run_sync_gate(
        temp.path(),
        archon_core::config::GateMode::Observe,
        crate::command::workflow_gate::GateId::WorkflowLintTaskFile,
        || evaluate_lint(temp.path(), &source, archon_core::config::GateMode::Observe),
    )
    .expect("a complete acceptance chain does not require a successor skeleton chain");

    disposition.require_allowed().unwrap();
    assert!(
        disposition
            .report()
            .contains("acceptance contract, lock, PRD digest, and host pin match"),
        "{}",
        disposition.report()
    );
    assert!(
        disposition
            .report()
            .contains("no skeleton file, lock, or pin exists"),
        "{}",
        disposition.report()
    );
}

#[test]
fn malformed_pin_is_operational_for_task_file_and_task_set_in_observe_mode() {
    let temp = tempfile::tempdir().unwrap();
    let task = write_task_file_lint_fixture(temp.path());
    let tasks = task.parent().unwrap();
    let pin_path = crate::command::workflow_task_set::acceptance_pin_path(temp.path(), tasks);
    std::fs::write(&pin_path, "not-json").unwrap();

    for (source, gate_id) in lint_sources(&task) {
        let error = crate::command::workflow_gate::run_sync_gate(
            temp.path(),
            archon_core::config::GateMode::Observe,
            gate_id,
            || evaluate_lint(temp.path(), &source, archon_core::config::GateMode::Observe),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("malformed or unstamped"), "{error}");
    }

    assert!(!crate::command::workflow_gate::shadow_log_path(temp.path()).exists());
}
