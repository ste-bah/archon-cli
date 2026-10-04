//! Binding legacy-debris rules and recovery interruption regressions (#271).
use super::round_three::frozen;
use super::*;
use archon_workflow::task_set_contract::{
    ACCEPTANCE_CONTRACT_FILE, ACCEPTANCE_LOCK_FILE, AcceptanceLock, TASK_SKELETON_LOCK_FILE,
};
const TXN: &str = "aabbccddeeff00112233445566778899";

#[test]
fn partial_legacy_staging_never_replaces_the_intact_contract() {
    for bytes in [
        b"".as_slice(),
        b"{\"schema_version\":",
        b"a complete but different candidate",
    ] {
        let set = frozen();
        let before = set.chain_bytes();
        let staged =
            sibling_transaction_path(&set.tasks.join(ACCEPTANCE_CONTRACT_FILE), TXN, "new");
        std::fs::write(&staged, bytes).unwrap();
        recover_interrupted_publish(&set.pin_path(), &set.tasks).unwrap();
        assert_eq!(
            set.chain_bytes(),
            before,
            "staging must not touch the live set"
        );
        assert!(!staged.exists());
        assert!(
            std::fs::read_to_string(recovery_log_path(&set.pin_path()))
                .unwrap()
                .contains("discard")
        );
    }
}

#[test]
fn verification_is_durable_before_legacy_evidence_is_consumed() {
    let set = frozen();
    let lock_path = set.tasks.join(ACCEPTANCE_LOCK_FILE);
    let mut lock: AcceptanceLock =
        serde_json::from_slice(&std::fs::read(&lock_path).unwrap()).unwrap();
    lock.digest = content_digest(b"not the contract");
    std::fs::copy(&lock_path, sibling_transaction_path(&lock_path, TXN, "old")).unwrap();
    std::fs::write(
        sibling_transaction_path(&lock_path, TXN, "new"),
        serde_json::to_vec(&lock).unwrap(),
    )
    .unwrap();
    let marker = set.pin_path().with_extension("publish-verification");
    let observed = marker.clone();
    super::super::journal::test_hooks::STEP.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |step| {
            if step == "before-recovery-log" {
                assert!(
                    observed.exists(),
                    "verification intent must survive consumed staging"
                );
                panic!("simulated recovery interruption");
            }
        }))
    });
    let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        recover_interrupted_publish(&set.pin_path(), &set.tasks)
    }));
    super::super::journal::test_hooks::STEP.with(|hook| *hook.borrow_mut() = None);
    assert!(interrupted.is_err());
    assert!(
        marker.exists(),
        "interruption must retain verification intent"
    );
    // Reproduce the point where ALL transaction debris has been consumed.
    for role in ["old", "new"] {
        let _ = std::fs::remove_file(sibling_transaction_path(&lock_path, TXN, role));
    }
    recover_interrupted_publish(&set.pin_path(), &set.tasks).unwrap();
    assert!(!lock_path.exists());
    assert!(!set.pin_path().exists());
    assert!(
        !marker.exists(),
        "clear only after verification/unfreeze finishes"
    );
    // Real process kills exercise cleanup and every move-aside boundary.
    for point in [
        "legacy-backups-removed",
        "unfreeze-moved-0",
        "unfreeze-moved-1",
        "unfreeze-moved-2",
        "unfreeze-moved-3",
        "legacy-verified",
    ] {
        let set = frozen();
        let lock_path = set.tasks.join(ACCEPTANCE_LOCK_FILE);
        std::fs::copy(&lock_path, sibling_transaction_path(&lock_path, TXN, "old")).unwrap();
        std::fs::write(&lock_path, b"interrupted legacy rollback").unwrap();
        let sidecar = set
            .pin_path()
            .parent()
            .unwrap()
            .join("check-sources")
            .join(set.pin_path().file_name().unwrap());
        std::fs::create_dir_all(sidecar.parent().unwrap()).unwrap();
        std::fs::write(&sidecar, b"old sidecar").unwrap();
        let evidence = set.project.path().join("crash-step");
        let status = Command::new(std::env::current_exe().unwrap())
            .args([&child_test_name(), "--exact", "--test-threads=1"])
            .env(CHILD_ROOT_ENV, set.project.path())
            .env("ARCHON_TEST_RECOVER_TASKS", "tasks/PRD-F")
            .env("ARCHON_TEST_RECOVER_CHILD", "1")
            .env(CRASH_ENV, point)
            .env(super::super::journal::CRASH_EVIDENCE_ENV, &evidence)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(
            killed_at_crash_point(status, point, &evidence),
            "missing step {point}"
        );
        assert!(
            set.pin_path()
                .with_extension("publish-verification")
                .exists(),
            "pending at {point}"
        );
        recover_interrupted_publish(&set.pin_path(), &set.tasks).unwrap();
        assert!(!lock_path.exists(), "{point}");
        assert!(!set.pin_path().exists(), "{point}");
        assert!(!sidecar.exists(), "{point}");
        assert!(
            !set.pin_path()
                .with_extension("publish-verification")
                .exists(),
            "{point}"
        );
    }
}

#[test]
fn every_interrupted_move_aside_boundary_finishes_unfreezing() {
    for boundary in 1..=4 {
        let set = frozen();
        let sidecar = set
            .pin_path()
            .parent()
            .unwrap()
            .join("check-sources")
            .join(set.pin_path().file_name().unwrap());
        std::fs::create_dir_all(sidecar.parent().unwrap()).unwrap();
        std::fs::write(&sidecar, b"sidecar").unwrap();
        let targets = [
            set.tasks.join(ACCEPTANCE_LOCK_FILE),
            set.tasks.join(TASK_SKELETON_LOCK_FILE),
            set.pin_path(),
            sidecar,
        ];
        for target in targets.iter().take(boundary) {
            let name = target.file_name().unwrap().to_string_lossy();
            std::fs::rename(
                target,
                target.with_file_name(format!(".{name}.unverified-{TXN}")),
            )
            .unwrap();
        }
        recover_interrupted_publish(&set.pin_path(), &set.tasks).unwrap();
        assert!(
            targets.iter().all(|path| !path.exists()),
            "boundary {boundary}: {targets:?}"
        );
    }
}

#[cfg(unix)]
#[test]
fn an_external_archon_store_with_an_existing_run_authorizes_and_recovers_receipts() {
    let project = tempfile::tempdir().unwrap();
    let external = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(external.path(), project.path().join(".archon")).unwrap();
    let tasks = project.path().join("tasks");
    std::fs::create_dir_all(&tasks).unwrap();
    let pin = crate::command::workflow_task_set::acceptance_pin_path(project.path(), &tasks);
    std::fs::create_dir_all(pin.parent().unwrap()).unwrap();
    let store = archon_workflow::WorkflowStore::project(project.path());
    let run = store.run_dir("existing");
    let state = run.join(crate::command::workflow_decompose::FIXED_DECOMPOSITION_STATE_PATH);
    std::fs::create_dir_all(state.parent().unwrap()).unwrap();
    std::fs::write(
        &state,
        serde_json::json!({"identity": {"task_root_identity": tasks}}).to_string(),
    )
    .unwrap();
    recover_interrupted_publish(&pin, &tasks).unwrap();
    let receipt = run.join("host-command-results/call/gate-envelope.json");
    crate::command::workflow_host_command_publish::authority_for_test(&pin, &tasks, &receipt)
        .unwrap();
    let scopes =
        crate::command::workflow_host_command_publish::receipt_scopes(&pin, &tasks).unwrap();
    assert_eq!(scopes.len(), 1);
    std::fs::write(&receipt, b"old receipt").unwrap();
    std::fs::write(
        sibling_transaction_path(&receipt, TXN, "old"),
        b"older receipt",
    )
    .unwrap();
    std::fs::write(
        sibling_transaction_path(&receipt, TXN, "new"),
        b"new receipt",
    )
    .unwrap();
    recover_interrupted_publish(&pin, &tasks).unwrap();
    assert_eq!(std::fs::read(&receipt).unwrap(), b"new receipt");
    // A link below the authority root must still be refused.
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), run.join("host-command-results/escape")).unwrap();
    assert!(
        crate::command::workflow_host_command_publish::authority_for_test(
            &pin,
            &tasks,
            &run.join("host-command-results/escape/gate-envelope.json")
        )
        .is_err()
    );
}

#[cfg(unix)]
#[test]
fn an_unrelated_termination_is_not_a_named_crash() {
    let status = Command::new("/bin/sh")
        .args(["-c", "kill -KILL $$"])
        .status()
        .unwrap();
    assert!(
        std::panic::catch_unwind(|| killed_at_crash_point(
            status,
            "unreached-step",
            &PathBuf::from("unreached-crash-evidence")
        ))
        .is_err(),
        "a termination without evidence of the named step must be refused"
    );
}
