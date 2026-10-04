//! Regression tests for the hostile second review of Issue 271.
use super::super::journal::{self, Journal, JournalEntry, JournalPaths, JournalState};
use super::*;

const TXN: &str = "00112233445566778899aabbccddeeff";

pub(super) fn fixture(s: &Scenario, state: JournalState, identical: bool) -> Journal {
    let entries = s
        .targets()
        .into_iter()
        .zip(NEW)
        .enumerate()
        .map(|(i, (target, bytes))| {
            if identical && i == 0 {
                std::fs::write(&target, bytes.unwrap()).unwrap();
            }
            let backup = target.exists().then(|| {
                let backup = sibling_transaction_path(&target, TXN, "old");
                std::fs::hard_link(&target, &backup).unwrap();
                backup
            });
            let staged = sibling_transaction_path(&target, TXN, "new");
            std::fs::write(&staged, bytes.unwrap()).unwrap();
            JournalEntry {
                target,
                staged,
                backup,
                written: content_digest(bytes.unwrap()),
            }
        })
        .collect();
    let mut j = Journal::new(TXN.into(), "test", entries);
    j.state = state;
    j.store(&JournalPaths::for_pin(&s.pin())).unwrap();
    j
}

fn recover_child(s: &Scenario, point: &str) -> bool {
    !Command::new(std::env::current_exe().unwrap())
        .args([&child_test_name(), "--exact", "--test-threads=1"])
        .env(CHILD_ROOT_ENV, s.root())
        .env("ARCHON_TEST_RECOVER_CHILD", "1")
        .env(CRASH_ENV, point)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap()
        .success()
}

#[test]
fn recovery_interruptions_preserve_identical_old_files_and_log() {
    for point in [
        "restored-0",
        "restored-1",
        "restored-2",
        "restored-3",
        "backup-cleaned-0",
        "backup-cleaned-1",
        "backup-cleaned-2",
        "backup-cleaned-3",
        "before-journal-removal",
        "before-recovery-log",
        "after-recovery-log",
    ] {
        let s = Scenario::new();
        let j = fixture(&s, JournalState::Applying, true);
        for entry in &j.entries {
            std::fs::rename(&entry.staged, &entry.target).unwrap();
        }
        assert!(
            recover_child(&s, point),
            "missing recovery interruption: {point}"
        );
        recover_interrupted_publish(&s.pin(), &s.tasks()).unwrap();
        let mut expected = OLD;
        expected[0] = NEW[0];
        assert!(s.is(expected), "{point}: {:?}", s.state());
        assert!(
            s.recovery_log().contains("rolled back"),
            "unlogged recovery at {point}"
        );
        assert!(s.debris().is_empty());
    }
}

#[test]
fn committed_recovery_really_moves_staging_and_survives_cleanup_interruptions() {
    for point in [
        "forwarded-0",
        "forwarded-1",
        "forwarded-2",
        "forwarded-3",
        "backup-cleaned-0",
        "backup-cleaned-3",
        "before-journal-removal",
        "before-recovery-log",
        "after-recovery-log",
    ] {
        let s = Scenario::new();
        fixture(&s, JournalState::Committed, false);
        assert!(!s.is(NEW));
        assert!(recover_child(&s, point));
        recover_interrupted_publish(&s.pin(), &s.tasks()).unwrap();
        assert!(s.is(NEW), "{point}: {:?}", s.state());
        assert!(
            s.recovery_log().contains("rolled forward"),
            "unlogged at {point}"
        );
    }
}

#[test]
fn commit_flush_error_never_starts_rollback_against_committed_journal() {
    journal_replacement_and_flush_interruptions_settle_one_set();
    let s = Scenario::new();
    let status = Command::new(std::env::current_exe().unwrap())
        .args([&child_test_name(), "--exact", "--test-threads=1"])
        .env(CHILD_ROOT_ENV, s.root())
        .env("ARCHON_TEST_COMMIT_FLUSH_ERROR", "1")
        .env(CRASH_ENV, "restored-0")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(!status.success());
    recover_interrupted_publish(&s.pin(), &s.tasks()).unwrap();
    assert!(s.is(NEW), "{:?}", s.state());
}

fn journal_replacement_and_flush_interruptions_settle_one_set() {
    for (point, committed) in [
        ("journal-replaced-Prepared", false),
        ("journal-flushed-Prepared", false),
        ("journal-replaced-Applying", false),
        ("journal-flushed-Applying", false),
        ("journal-replaced-Committed", true),
        ("journal-flushed-Committed", true),
        ("commit-backup-cleaned-0", true),
        ("commit-backup-cleaned-1", true),
        ("commit-backup-cleaned-2", true),
        ("commit-before-journal-removal", true),
    ] {
        let s = Scenario::new();
        assert!(s.publish_killed_at(point));
        recover_interrupted_publish(&s.pin(), &s.tasks()).unwrap();
        assert!(s.is(if committed { NEW } else { OLD }), "{point}");
    }
}

#[test]
fn committed_missing_staging_must_refuse_instead_of_accepting_mixed_set() {
    let s = Scenario::new();
    let j = fixture(&s, JournalState::Committed, false);
    std::fs::remove_file(&j.entries[0].staged).unwrap();
    let before = s.state();
    assert!(recover_interrupted_publish(&s.pin(), &s.tasks()).is_err());
    assert_eq!(s.state(), before);
    assert!(j.entries[0].backup.as_ref().unwrap().exists());
}

#[test]
fn legacy_absent_target_and_interrupted_rollback_are_preserved_as_corruption() {
    for rollback in [false, true] {
        let s = Scenario::new();
        let [a, b, c, _] = s.targets();
        if rollback {
            std::fs::rename(&b, sibling_transaction_path(&b, TXN, "old")).unwrap();
            std::fs::write(&b, b"new-b").unwrap();
            // a was already restored; all remaining staging was discarded.
        } else {
            std::fs::write(&c, b"new-c").unwrap();
            std::fs::write(sibling_transaction_path(&a, TXN, "new"), b"new-a").unwrap();
        }
        let before = s.state();
        let debris = s.debris();
        let error =
            recover_interrupted_publish(&s.pin(), &s.tasks()).expect_err("ambiguous legacy set");
        assert!(error.to_string().contains(TXN), "{error}");
        assert_eq!(s.state(), before);
        assert_eq!(s.debris(), debris);
    }
}

#[test]
fn crafted_foreign_traversal_and_symlink_targets_cannot_delete_victims() {
    let s = Scenario::new();
    let outside = tempfile::tempdir().unwrap();
    let victim = outside.path().join("victim.json");
    std::fs::write(&victim, b"victim").unwrap();
    let foreign_gate = s
        .root()
        .join(".archon/workflows/foreign-run/host-command-results/foreign-call/gate-envelope.json");
    let other_set = tempfile::tempdir().unwrap();
    crate::command::workflow_task_set::create_dir_all_durably(foreign_gate.parent().unwrap())
        .unwrap();
    crate::command::workflow_host_command_publish::authority_for_test(
        &s.pin(),
        other_set.path(),
        &foreign_gate,
    )
    .unwrap();
    std::fs::write(&foreign_gate, b"victim").unwrap();
    let mut targets = vec![
        victim.clone(),
        s.tasks().join("../../victim.json"),
        foreign_gate,
    ];
    std::fs::write(s.root().join("victim.json"), b"victim").unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(outside.path(), s.tasks().join("escape")).unwrap();
        targets.push(s.tasks().join("escape/victim.json"));
    }
    for target in targets {
        let j = Journal {
            schema_version: 1,
            transaction: TXN.into(),
            state: JournalState::Applying,
            remedy: "test".into(),
            entries: vec![JournalEntry {
                staged: sibling_transaction_path(&target, TXN, "new"),
                backup: None,
                written: content_digest(b"victim"),
                target: target.clone(),
            }],
        };
        j.store(&JournalPaths::for_pin(&s.pin())).unwrap();
        assert!(
            recover_interrupted_publish(&s.pin(), &s.tasks()).is_err(),
            "accepted {}",
            target.display()
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"victim");
    }
}

#[test]
fn first_publish_flushes_every_new_directory_parent() {
    let s = Scenario::new();
    std::fs::remove_dir_all(s.pin().parent().unwrap()).unwrap();
    journal::test_hooks::SYNCS.with(|p| p.borrow_mut().clear());
    publish(&s.pin(), &s.new_files()).unwrap();
    let synced = journal::test_hooks::SYNCS.with(|p| p.borrow().clone());
    assert!(synced.contains(&s.root().join(".archon")), "{synced:?}");
    assert!(synced.contains(&s.root().to_path_buf()), "{synced:?}");
}

#[test]
fn decomposition_and_launch_hold_publish_lock_through_reads() {
    for point in ["decomposition-read", "launch-read"] {
        let s = Scenario::new();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let root = s.root().to_path_buf();
        let tasks = s.tasks();
        let reader = std::thread::spawn(move || {
            journal::test_hooks::STEP.with(|h| {
                *h.borrow_mut() = Some(Box::new(move |step| {
                    if step == point {
                        ready_tx.send(()).unwrap();
                        release_rx.recv().unwrap();
                    }
                }))
            });
            if point == "decomposition-read" {
                crate::command::workflow_decompose_frozen_chain::frozen_chain_snapshot(
                    &root,
                    &root.join("prd.md"),
                    &tasks,
                )
                .unwrap();
            } else {
                crate::command::workflow_task_set::republish::refuse_unaccepted_launch(
                    &root, &tasks,
                )
                .unwrap();
            }
        });
        ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let pin = s.pin();
        let files = s.new_files();
        let writer = std::thread::spawn(move || {
            done_tx
                .send(begin(&pin, &files).unwrap().commit().unwrap())
                .unwrap();
        });
        let blocked = done_rx.recv_timeout(Duration::from_millis(200)).is_err();
        release_tx.send(()).unwrap();
        reader.join().unwrap();
        writer.join().unwrap();
        assert!(blocked, "publisher interleaved during {point}");
    }
}

#[test]
fn recovery_log_failure_keeps_the_durable_decision_for_retry() {
    let s = Scenario::new();
    fixture(&s, JournalState::Committed, false);
    let log = recovery_log_path(&s.pin());
    std::fs::create_dir(&log).unwrap();
    assert!(recover_interrupted_publish(&s.pin(), &s.tasks()).is_err());
    assert!(s.pin().with_extension("publish-journal").exists());
    std::fs::remove_dir(log).unwrap();
    recover_interrupted_publish(&s.pin(), &s.tasks()).unwrap();
    assert!(s.is(NEW));
    assert!(s.recovery_log().contains("rolled forward"));
}

#[test]
fn commit_cleanup_flushes_backup_directories_before_forgetting_decision() {
    let s = Scenario::new();
    let transaction = begin(&s.pin(), &s.new_files()).unwrap();
    journal::test_hooks::SYNCS.with(|p| p.borrow_mut().clear());
    let tasks = s.tasks();
    journal::test_hooks::STEP.with(|h| {
        *h.borrow_mut() = Some(Box::new(move |step| {
            if step == "commit-before-journal-removal" {
                let synced = journal::test_hooks::SYNCS.with(|p| p.borrow().clone());
                assert!(
                    synced.contains(&tasks),
                    "backup unlinks must be durable: {synced:?}"
                );
            }
        }))
    });
    transaction.commit().unwrap();
    journal::test_hooks::STEP.with(|h| *h.borrow_mut() = None);
}
