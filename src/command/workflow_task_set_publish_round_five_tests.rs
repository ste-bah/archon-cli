//! Fifth-review regressions for legacy crash decisions.
use super::round_three::frozen;
use super::*;
use archon_workflow::task_set_contract::{ACCEPTANCE_LOCK_FILE, TASK_SKELETON_LOCK_FILE};
const TXN: &str = "aabbccddeeff00112233445566778899";

#[test]
fn first_freeze_killed_before_pin_verifies_discarded_staging() {
    let set = frozen();
    let pin = set.pin_path();
    std::fs::rename(&pin, sibling_transaction_path(&pin, TXN, "new")).unwrap();
    // Old publisher only backed up existing targets: a first freeze has no .old.
    recover_interrupted_publish(&pin, &set.tasks).unwrap();
    assert!(
        !set.tasks.join(ACCEPTANCE_LOCK_FILE).exists(),
        "incomplete first freeze must unfreeze"
    );
    crate::command::workflow_task_set::republish::refuse_unaccepted_launch(
        set.project.path(),
        &set.tasks,
    )
    .unwrap();
}

#[test]
fn unchanged_backup_does_not_discard_complete_legacy_staging() {
    let s = Scenario::new();
    let [a, b, _, _] = s.targets();
    std::fs::copy(&a, sibling_transaction_path(&a, TXN, "old")).unwrap();
    std::fs::copy(&b, sibling_transaction_path(&b, TXN, "old")).unwrap();
    std::fs::write(sibling_transaction_path(&b, TXN, "new"), b"new-b").unwrap();
    recover_interrupted_publish(&s.pin(), &s.tasks()).unwrap();
    assert_eq!(std::fs::read(&a).unwrap(), b"old-a");
    assert_eq!(
        std::fs::read(&b).unwrap(),
        b"new-b",
        "unchanged a is already equal, not rollback evidence"
    );
}

#[test]
fn interrupted_legacy_rollback_restores_remaining_backups() {
    let s = Scenario::new();
    let [_, b, _, _] = s.targets();
    std::fs::rename(&b, sibling_transaction_path(&b, TXN, "old")).unwrap();
    recover_interrupted_publish(&s.pin(), &s.tasks()).unwrap();
    assert_eq!(
        std::fs::read(&b).ok(),
        Some(b"old-b".to_vec()),
        "restore, never delete a remaining rollback backup"
    );
    // The locks can already be restored while a task body's .old remains.
    let set = frozen();
    let lock = set.tasks.join(ACCEPTANCE_LOCK_FILE);
    let body = set.tasks.join("TASK-F-001.md");
    let old = std::fs::read(&body).unwrap();
    std::fs::copy(&lock, sibling_transaction_path(&lock, TXN, "old")).unwrap();
    std::fs::copy(&body, sibling_transaction_path(&body, TXN, "old")).unwrap();
    std::fs::write(&body, b"half rolled-back body").unwrap();
    recover_interrupted_publish(&set.pin_path(), &set.tasks).unwrap();
    assert_eq!(std::fs::read(body).unwrap(), old);
    assert!(lock.exists(), "the restored chain stays frozen");
}

#[test]
fn legacy_roll_decision_is_persisted_before_the_first_rename() {
    let s = Scenario::new();
    let [a, b, _, _] = s.targets();
    for target in [&a, &b] {
        std::fs::copy(target, sibling_transaction_path(target, TXN, "old")).unwrap();
        // a is unchanged; this must not flip a saved forward decision on retry.
        std::fs::write(
            sibling_transaction_path(target, TXN, "new"),
            if target == &a { b"old-a" } else { b"new-b" },
        )
        .unwrap();
    }
    let marker = s.pin().with_extension("publish-verification");
    let observed = marker.clone();
    super::super::journal::test_hooks::STEP.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |step| {
            if step == "before-recovery-log" {
                let value: serde_json::Value =
                    serde_json::from_slice(&std::fs::read(&observed).unwrap()).unwrap();
                assert_eq!(
                    value["decisions"][TXN], "forward",
                    "save the decision before consuming evidence"
                );
                panic!("interrupted recovery");
            }
        }));
    });
    let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        recover_interrupted_publish(&s.pin(), &s.tasks())
    }));
    super::super::journal::test_hooks::STEP.with(|hook| *hook.borrow_mut() = None);
    assert!(interrupted.is_err());
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&marker).unwrap()).unwrap();
    assert_eq!(value["decisions"][TXN], "forward");
    recover_interrupted_publish(&s.pin(), &s.tasks()).unwrap();
    assert_eq!(std::fs::read(b).unwrap(), b"new-b");
    // Process deaths at the individual rename/restore boundaries must use
    // the saved decision, including an unchanged first file.
    for (point, forward) in [
        ("legacy-renamed-0", true),
        ("legacy-renamed-1", true),
        ("legacy-restored-0", false),
    ] {
        let s = Scenario::new();
        let [a, b, _, _] = s.targets();
        for target in [&a, &b] {
            std::fs::copy(target, sibling_transaction_path(target, TXN, "old")).unwrap();
            if forward {
                std::fs::write(
                    sibling_transaction_path(target, TXN, "new"),
                    if target == &a { b"old-a" } else { b"new-b" },
                )
                .unwrap();
            } else if target == &b {
                std::fs::write(target, b"new-b").unwrap();
            }
        }
        let evidence = s.root().join("crash-step");
        let status = Command::new(std::env::current_exe().unwrap())
            .args([&child_test_name(), "--exact", "--test-threads=1"])
            .env(CHILD_ROOT_ENV, s.root())
            .env("ARCHON_TEST_RECOVER_TASKS", "tasks/set")
            .env("ARCHON_TEST_RECOVER_CHILD", "1")
            .env(CRASH_ENV, point)
            .env(super::super::journal::CRASH_EVIDENCE_ENV, &evidence)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(killed_at_crash_point(status, point, &evidence));
        recover_interrupted_publish(&s.pin(), &s.tasks()).unwrap();
        assert_eq!(std::fs::read(&a).unwrap(), b"old-a", "{point}");
        assert_eq!(
            std::fs::read(&b).unwrap(),
            if forward { b"new-b" } else { b"old-b" },
            "{point}"
        );
        assert!(s.debris().is_empty());
    }
}

#[test]
fn old_inspection_files_do_not_authorize_later_tampering() {
    let set = frozen();
    let lock = set.tasks.join(ACCEPTANCE_LOCK_FILE);
    std::fs::copy(&lock, sibling_transaction_path(&lock, TXN, "old")).unwrap();
    std::fs::write(&lock, b"crash debris").unwrap();
    recover_interrupted_publish(&set.pin_path(), &set.tasks).unwrap();
    // Recreate the intact chain, leaving the inspection files from the recovery.
    crate::command::workflow_task_set::republish::test_fixture::write_chain(
        set.project.path(),
        &set.tasks,
        &set.contract(),
        archon_workflow::task_set_contract::FreezeGateMode::Observe,
        0,
    );
    std::fs::remove_file(&lock).unwrap();
    recover_interrupted_publish(&set.pin_path(), &set.tasks).unwrap();
    assert!(
        set.pin_path().exists(),
        "historical inspection files must grant no new recovery"
    );
    assert!(set.tasks.join(TASK_SKELETON_LOCK_FILE).exists());
    assert!(
        crate::command::workflow_task_set::republish::refuse_unaccepted_launch(
            set.project.path(),
            &set.tasks
        )
        .is_err(),
        "tampering outside recovery stays refused"
    );
}
