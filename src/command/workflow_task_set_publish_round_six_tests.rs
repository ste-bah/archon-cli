//! Completed legacy publication with unchanged hard-linked targets.
use super::super::recover::RecoveryOutcome;
use super::round_three::frozen;
use super::*;

#[test]
fn completed_legacy_publish_with_unchanged_file_rolls_forward() {
    let set = frozen();
    let txn = "aabbccddeeff00112233445566778899";
    let unchanged = set.tasks.join("TASK-F-001.md");
    let unchanged_backup = sibling_transaction_path(&unchanged, txn, "old");
    std::fs::hard_link(&unchanged, &unchanged_backup).unwrap();
    let changed = set.tasks.join("publication.txt");
    let changed_backup = sibling_transaction_path(&changed, txn, "old");
    std::fs::write(&changed, b"before publication").unwrap();
    std::fs::hard_link(&changed, &changed_backup).unwrap();
    let staged = sibling_transaction_path(&changed, txn, "new");
    std::fs::write(&staged, b"completed publication").unwrap();
    std::fs::rename(&staged, &changed).unwrap();
    let chain = set.chain_bytes();
    let report = recover_interrupted_publish(&set.pin_path(), &set.tasks).unwrap();
    assert_eq!(
        report.events[0].outcome,
        RecoveryOutcome::RolledForward,
        "a verified completed chain must keep the publication"
    );
    assert_eq!(std::fs::read(changed).unwrap(), b"completed publication");
    assert_eq!(set.chain_bytes(), chain);
    assert!(!unchanged_backup.exists());
    assert!(!changed_backup.exists());
}
