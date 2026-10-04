//! Regression tests for the third review of Issue 271.
use super::super::journal::{self, Journal, JournalEntry, JournalPaths, JournalState};
use super::super::recover::RecoveryOutcome;
use super::*;
use archon_workflow::task_set_contract::{
    ACCEPTANCE_LOCK_FILE, AcceptanceLock, TASK_SKELETON_LOCK_FILE,
};

const TXN: &str = "aabbccddeeff00112233445566778899";

fn frozen() -> crate::command::workflow_task_set::republish::test_fixture::FrozenSet {
    crate::command::workflow_task_set::republish::test_fixture::frozen_set(&[(
        "AC-F-001",
        "jq -e '.a == true' out.json",
        true,
    )])
}

fn hidden_names(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with('.'))
        .collect()
}

/// Legacy debris whose roll-forward leaves a chain that does not verify is
/// never refused: the set is treated as not frozen (its locks and pin moved
/// aside, kept for inspection) so the workflow re-freezes it.
#[test]
fn legacy_debris_that_breaks_the_chain_unfreezes_the_set_for_refreeze() {
    let set = frozen();
    let lock_path = set.tasks.join(ACCEPTANCE_LOCK_FILE);
    let mut lock: AcceptanceLock =
        serde_json::from_slice(&std::fs::read(&lock_path).unwrap()).unwrap();
    lock.digest = content_digest(b"bytes the contract does not hold");
    std::fs::write(
        sibling_transaction_path(&lock_path, TXN, "new"),
        serde_json::to_vec_pretty(&lock).unwrap(),
    )
    .unwrap();
    let report = recover_interrupted_publish(&set.pin_path(), &set.tasks).unwrap();
    assert!(!lock_path.exists(), "unverified lock must not stay frozen");
    assert!(!set.tasks.join(TASK_SKELETON_LOCK_FILE).exists());
    assert!(!set.pin_path().exists());
    assert!(
        hidden_names(&set.tasks)
            .iter()
            .any(|name| name.contains("unverified")),
        "the unverified chain is kept for inspection: {:?}",
        hidden_names(&set.tasks)
    );
    assert!(
        report
            .events
            .iter()
            .any(|event| event.outcome == RecoveryOutcome::Unfrozen),
        "{:?}",
        report.events
    );
    let log = std::fs::read_to_string(recovery_log_path(&set.pin_path())).unwrap();
    assert!(log.contains("unfrozen"), "{log}");
    crate::command::workflow_task_set::republish::refuse_unaccepted_launch(
        set.project.path(),
        &set.tasks,
    )
    .unwrap();
    let snapshot = crate::command::workflow_decompose_frozen_chain::frozen_chain_snapshot(
        set.project.path(),
        &set.prd,
        &set.tasks,
    )
    .unwrap();
    assert!(!snapshot.acceptance && !snapshot.skeleton);
}

/// A backup an older binary failed to remove after a good publish is
/// dropped; the chain still verifies and stays frozen.
#[test]
fn a_leftover_legacy_backup_on_a_verified_chain_keeps_it_frozen() {
    let set = frozen();
    let lock_path = set.tasks.join(TASK_SKELETON_LOCK_FILE);
    let backup = sibling_transaction_path(&lock_path, TXN, "old");
    std::fs::write(&backup, b"the version before the good publish").unwrap();
    let before = set.chain_bytes();
    let report = recover_interrupted_publish(&set.pin_path(), &set.tasks).unwrap();
    assert!(!backup.exists());
    assert_eq!(set.chain_bytes(), before);
    assert!(
        report
            .events
            .iter()
            .all(|event| event.outcome != RecoveryOutcome::Unfrozen),
        "{:?}",
        report.events
    );
    let snapshot = crate::command::workflow_decompose_frozen_chain::frozen_chain_snapshot(
        set.project.path(),
        &set.prd,
        &set.tasks,
    )
    .unwrap();
    assert!(snapshot.acceptance && snapshot.skeleton);
}

/// Only the directories created and the first one that already existed are
/// flushed: a standard Windows user cannot open `C:\` or `C:\Users` for write.
#[test]
fn durable_mkdir_flushes_only_created_directories_and_their_first_parent() {
    let temp = tempfile::tempdir().unwrap();
    let existing = temp.path().join("existing");
    std::fs::create_dir(&existing).unwrap();
    let deepest = existing.join("a/b/c");
    journal::test_hooks::SYNCS.with(|p| p.borrow_mut().clear());
    crate::command::workflow_task_set::create_dir_all_durably(&deepest).unwrap();
    let mut synced = journal::test_hooks::SYNCS.with(|p| p.borrow().clone());
    synced.sort();
    let mut expected = vec![
        existing.clone(),
        existing.join("a"),
        existing.join("a/b"),
        deepest.clone(),
    ];
    expected.sort();
    assert_eq!(synced, expected);
}

/// A symlinked project root or task root is a legitimate setup: publication
/// and recovery resolve it once instead of refusing it.
#[cfg(unix)]
#[test]
fn symlinked_project_and_task_roots_publish_and_recover() {
    let real = tempfile::tempdir().unwrap();
    let links = tempfile::tempdir().unwrap();
    let project = links.path().join("project");
    std::os::unix::fs::symlink(real.path(), &project).unwrap();
    let real_tasks = real.path().join("task-sets/real");
    std::fs::create_dir_all(&real_tasks).unwrap();
    let tasks = project.join("tasks");
    std::os::unix::fs::symlink(&real_tasks, &tasks).unwrap();
    let pin = crate::command::workflow_task_set::acceptance_pin_path(&project, &tasks);
    let files = vec![
        (tasks.join("a.json"), b"new-a".to_vec()),
        (pin.clone(), b"new-pin".to_vec()),
    ];
    publish_files_atomically(&pin, &tasks, &files, "test").unwrap();
    assert_eq!(std::fs::read(real_tasks.join("a.json")).unwrap(), b"new-a");
    recover_interrupted_publish(&pin, &tasks).unwrap();
    crate::command::workflow_task_set::republish::refuse_unaccepted_launch(&project, &tasks)
        .unwrap();
}

/// The chain lock file is reserved even where its directory is authorized.
#[test]
fn a_journal_target_naming_the_chain_lock_is_reserved() {
    let s = Scenario::new();
    let paths = JournalPaths::for_pin(&s.pin());
    let chain_lock = s.pin().with_extension("chain.lock");
    std::fs::write(&chain_lock, b"").unwrap();
    let journal = Journal {
        schema_version: 1,
        transaction: TXN.into(),
        state: JournalState::Applying,
        remedy: "test".into(),
        entries: vec![JournalEntry {
            staged: sibling_transaction_path(&chain_lock, TXN, "new"),
            backup: None,
            written: content_digest(b""),
            target: chain_lock.clone(),
        }],
    };
    let pin_dir = s.pin().parent().unwrap().to_path_buf();
    let error = journal
        .validate(&paths, &[(pin_dir, None)])
        .expect_err("the chain lock is reserved");
    assert!(error.to_string().contains("reserved"), "{error}");
}

/// An unfreeze a crash cut short (the acceptance lock already moved aside)
/// is finished by the next recovery; it never leaves a half-frozen chain.
#[test]
fn an_interrupted_unfreeze_is_finished_by_the_next_recovery() {
    let set = frozen();
    let lock_path = set.tasks.join(ACCEPTANCE_LOCK_FILE);
    std::fs::rename(
        &lock_path,
        set.tasks
            .join(format!(".{ACCEPTANCE_LOCK_FILE}.unverified-{TXN}")),
    )
    .unwrap();
    recover_interrupted_publish(&set.pin_path(), &set.tasks).unwrap();
    assert!(!set.tasks.join(TASK_SKELETON_LOCK_FILE).exists());
    assert!(!set.pin_path().exists());
    let snapshot = crate::command::workflow_decompose_frozen_chain::frozen_chain_snapshot(
        set.project.path(),
        &set.prd,
        &set.tasks,
    )
    .unwrap();
    assert!(!snapshot.acceptance && !snapshot.skeleton);
}
