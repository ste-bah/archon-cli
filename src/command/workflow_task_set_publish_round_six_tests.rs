//! Completed legacy publication with unchanged hard-linked targets.
use super::super::recover::RecoveryOutcome;
use super::round_three::frozen;
use super::*;

#[test]
fn completed_legacy_publish_with_unchanged_file_preserves_live_targets() {
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
        RecoveryOutcome::Preserved,
        "without transaction-bound written digests, preserve live bytes without claiming a proved commit"
    );
    assert_eq!(std::fs::read(changed).unwrap(), b"completed publication");
    assert_eq!(set.chain_bytes(), chain);
    assert!(!unchanged_backup.exists());
    assert!(!changed_backup.exists());
}

#[test]
fn issue300_body_only_debris_ignores_unrelated_chain_failure() {
    let set = frozen();
    let body = set.tasks.join("publication.txt");
    std::fs::write(&body, b"published body").unwrap();
    std::fs::write(
        sibling_transaction_path(&body, "aabbccddeeff00112233445566778899", "old"),
        b"stale body",
    )
    .unwrap();
    std::fs::write(
        set.tasks
            .join(archon_workflow::task_set_contract::ACCEPTANCE_LOCK_FILE),
        b"broken unrelated lock",
    )
    .unwrap();
    let report = recover_interrupted_publish(&set.pin_path(), &set.tasks).unwrap();
    assert_eq!(
        std::fs::read(body).unwrap(),
        b"published body",
        "global failure cannot authorize a body rollback"
    );
    assert!(
        set.pin_path().exists(),
        "body debris cannot unfreeze an unrelated chain"
    );
    assert_eq!(report.events.len(), 1);
    let log = std::fs::read_to_string(recovery_log_path(&set.pin_path())).unwrap();
    let receipt: serde_json::Value = log
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .last()
        .unwrap();
    assert_eq!(
        receipt["file_evidence"][set.tasks.join("publication.txt").display().to_string()],
        content_digest(b"published body")
    );
}

#[test]
fn issue300_stale_backup_cannot_replace_a_later_receipted_publish() {
    let set = frozen();
    let body = set.tasks.join("publication.txt");
    let receipt_path = set.tasks.join("publication-receipt.json");
    let bytes = b"later receipted body";
    let receipt = archon_workflow::PublicationReceiptV1 {
        schema_version: archon_workflow::PUBLICATION_RECEIPT_SCHEMA_VERSION,
        call_id: "later-publication".into(),
        command_id: "land-task-body".into(),
        entries: vec![archon_workflow::PublishedArtifactReceipt {
            relative_path: "publication.txt".into(),
            destination_path: body.display().to_string(),
            byte_len: bytes.len() as u64,
            blake3: content_digest(bytes),
            prior_blake3: None,
        }],
        committed_at: "2026-10-06T00:00:00Z".into(),
    };
    let receipt_bytes = serde_json::to_vec(&receipt).unwrap();
    publish_files_atomically(
        &set.pin_path(),
        &set.tasks,
        &[
            (body.clone(), bytes.to_vec()),
            (receipt_path.clone(), receipt_bytes.clone()),
        ],
        "test",
    )
    .unwrap();
    let backup = sibling_transaction_path(&body, "aabbccddeeff00112233445566778899", "old");
    std::fs::write(&backup, b"older publication").unwrap();
    std::fs::write(
        set.tasks
            .join(archon_workflow::task_set_contract::ACCEPTANCE_LOCK_FILE),
        b"unrelated corruption",
    )
    .unwrap();
    recover_interrupted_publish(&set.pin_path(), &set.tasks).unwrap();
    assert_eq!(std::fs::read(body).unwrap(), b"later receipted body");
    assert!(!backup.exists());
    assert_eq!(std::fs::read(receipt_path).unwrap(), receipt_bytes);
}

#[test]
fn issue300_mid_rollback_retry_does_not_replace_restored_or_newer_targets() {
    let set = frozen();
    let txn = "aabbccddeeff00112233445566778899";
    let a = set.tasks.join("a.txt");
    let b = set.tasks.join("b.txt");
    let c = set.tasks.join("c.txt");
    std::fs::write(&a, b"current a").unwrap();
    for target in [&a, &b, &c] {
        std::fs::write(sibling_transaction_path(target, txn, "old"), b"prior").unwrap();
    }
    // A missing b is transaction-local evidence for restoring b, never a.
    let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        super::super::journal::test_hooks::STEP.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(|step| {
                if step.starts_with("legacy-restored-") {
                    panic!("crash after first restore");
                }
            }))
        });
        recover_interrupted_publish(&set.pin_path(), &set.tasks).unwrap();
    }));
    super::super::journal::test_hooks::STEP.with(|hook| *hook.borrow_mut() = None);
    assert!(crashed.is_err());
    assert_eq!(
        std::fs::read(&a).unwrap(),
        b"current a",
        "rollback only restores the missing target"
    );
    std::fs::write(&b, b"later b").unwrap();
    std::fs::write(&c, b"later c").unwrap();
    recover_interrupted_publish(&set.pin_path(), &set.tasks).unwrap();
    assert_eq!(std::fs::read(a).unwrap(), b"current a");
    assert_eq!(std::fs::read(b).unwrap(), b"later b");
    assert_eq!(std::fs::read(c).unwrap(), b"later c");
}

#[test]
fn issue300_old_rollback_marker_without_file_evidence_preserves_later_bytes() {
    let set = frozen();
    let txn = "aabbccddeeff00112233445566778899";
    let body = set.tasks.join("publication.txt");
    std::fs::write(&body, b"later publication").unwrap();
    std::fs::write(sibling_transaction_path(&body, txn, "old"), b"stale backup").unwrap();
    std::fs::write(
        set.pin_path().with_extension("publish-verification"),
        serde_json::json!({"transaction":txn,"decisions":{txn:"rollback"}}).to_string(),
    )
    .unwrap();
    recover_interrupted_publish(&set.pin_path(), &set.tasks).unwrap();
    assert_eq!(
        std::fs::read(body).unwrap(),
        b"later publication",
        "a bare rollback decision cannot authorize replacing a live target"
    );
}

fn preserved_rollback_report(bound: bool, partial: bool) {
    let set = frozen();
    let txn = "aabbccddeeff00112233445566778899";
    let body = set.tasks.join("publication.txt");
    std::fs::write(&body, b"later publication").unwrap();
    std::fs::write(sibling_transaction_path(&body, txn, "old"), b"stale backup").unwrap();
    let missing = set.tasks.join("missing.txt");
    if partial {
        std::fs::write(sibling_transaction_path(&missing, txn, "old"), b"prior").unwrap();
    }
    let mut marker = serde_json::json!({"transaction":txn,"decisions":{txn:"rollback"}});
    if bound {
        marker["written"] =
            serde_json::json!({txn:{body.display().to_string():content_digest(b"older write")}});
    }
    std::fs::write(
        set.pin_path().with_extension("publish-verification"),
        marker.to_string(),
    )
    .unwrap();
    let report = recover_interrupted_publish(&set.pin_path(), &set.tasks).unwrap();
    let event = &report.events[0];
    assert_eq!(
        event.outcome,
        if partial {
            RecoveryOutcome::RolledBack
        } else {
            RecoveryOutcome::Preserved
        }
    );
    assert!(
        event.files.contains(&body),
        "preserved paths must be reported: {event:?}"
    );
    let reason = if bound {
        "changed since"
    } else {
        "no transaction-bound"
    };
    let detail = event.detail.as_deref().unwrap();
    assert!(
        detail.contains("preserved") && detail.contains(reason),
        "{detail}"
    );
    assert!(detail.contains(&body.display().to_string()), "{detail}");
    let log = std::fs::read_to_string(recovery_log_path(&set.pin_path())).unwrap();
    let logged: serde_json::Value = log
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .find(|event: &serde_json::Value| {
            event["transaction"] == txn && event["source"] == "legacy"
        })
        .unwrap();
    assert_eq!(
        logged["outcome"],
        if partial {
            "rolled back"
        } else {
            "preserved live targets; unbound legacy backups discarded"
        }
    );
    assert_eq!(logged["detail"], detail);
    assert!(
        logged["files"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!(body))
    );
    assert_eq!(std::fs::read(body).unwrap(), b"later publication");
}
#[test]
fn review300_bare_rollback_reports_preservation() {
    preserved_rollback_report(false, false);
}
#[test]
fn review300_changed_digest_reports_preservation() {
    preserved_rollback_report(true, false);
}
#[test]
fn review300_partial_rollback_reports_each_preserved_path() {
    preserved_rollback_report(false, true);
}
