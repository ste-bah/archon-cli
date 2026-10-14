use super::*;
use archon_workflow::WorkflowV2BranchOutcome;

fn evidence(case: &str, before: impl std::fmt::Display, after: impl std::fmt::Display) {
    println!("EVIDENCE #283 {case} BEFORE: {before} AFTER: {after}");
}

// Expected before running: saving a replacement migrates a flat legacy
// superseded file under the item-specific archive and retains its bytes.
#[test]
fn evidence_283_save_migrates_flat_archive_and_preserves_superseded_record() {
    let temp = tempfile::tempdir().unwrap();
    let v2 = WorkflowV2ResultStore::new(temp.path());
    let initial = outcome("T-A", WorkflowV2Status::Accepted, "old-input", true);
    v2.save_branch_outcome(CALL, &initial).unwrap();
    let current = v2.branch_outcome_path(CALL, &initial.item_id);
    let call_dir = current.parent().unwrap().to_path_buf();
    let flat_archive = call_dir.join("superseded/legacy.json");
    std::fs::create_dir_all(flat_archive.parent().unwrap()).unwrap();
    let legacy = outcome("T-A", WorkflowV2Status::Noop, "legacy-input", false);
    let legacy_bytes = serde_json::to_vec(&legacy).unwrap();
    std::fs::write(&flat_archive, &legacy_bytes).unwrap();
    let before = format!(
        "flat={} current={}",
        flat_archive.exists(),
        std::fs::read(&current).unwrap().len()
    );

    let replacement = outcome("T-A", WorkflowV2Status::Noop, "new-input", false);
    v2.save_branch_outcome(CALL, &replacement).unwrap();
    let item_archive = call_dir
        .join("superseded")
        .join(branch::branch_component(&initial.item_id))
        .join("legacy.json");
    let separately_read = std::fs::read(&item_archive).unwrap();
    let saved_current: WorkflowV2BranchOutcome = serde_json::from_slice(
        &std::fs::read(&v2.branch_outcome_path(CALL, &replacement.item_id)).unwrap(),
    )
    .unwrap();
    assert!(!flat_archive.exists());
    assert_eq!(separately_read, legacy_bytes);
    assert_eq!(saved_current, replacement);
    evidence(
        "flat-archive-migrates-on-save",
        before,
        format!(
            "flat_exists={}; item_archive={}; bytes_preserved={}; current_status={:?}",
            flat_archive.exists(),
            item_archive.display(),
            separately_read == legacy_bytes,
            saved_current.status
        ),
    );
}

// Expected before running: distinct call/item ids with the same sanitized
// spelling persist to distinct hashed paths and load their own records.
#[test]
fn evidence_283_sanitized_collisions_use_distinct_hashed_files() {
    let temp = tempfile::tempdir().unwrap();
    let v2 = WorkflowV2ResultStore::new(temp.path());
    let slash = outcome("a/b", WorkflowV2Status::Accepted, "slash-input", true);
    let underscore = outcome("a_b", WorkflowV2Status::Noop, "underscore-input", false);
    let slash_path = v2.branch_outcome_path(CALL, &slash.item_id);
    let underscore_path = v2.branch_outcome_path(CALL, &underscore.item_id);
    let before = format!(
        "slash_exists={}; underscore_exists={}; paths_equal={}",
        slash_path.exists(),
        underscore_path.exists(),
        slash_path == underscore_path
    );
    v2.save_branch_outcome(CALL, &slash).unwrap();
    v2.save_branch_outcome(CALL, &underscore).unwrap();
    let slash_disk = std::fs::read(&slash_path).unwrap();
    let underscore_disk = std::fs::read(&underscore_path).unwrap();
    let slash_loaded = v2
        .load_branch_outcome(CALL, &slash.item_id)
        .unwrap()
        .unwrap();
    let underscore_loaded = v2
        .load_branch_outcome(CALL, &underscore.item_id)
        .unwrap()
        .unwrap();
    assert_ne!(slash_path, underscore_path);
    assert_eq!(slash_loaded, slash);
    assert_eq!(underscore_loaded, underscore);
    assert_ne!(slash_disk, underscore_disk);
    evidence(
        "sanitized-id-collision",
        before,
        format!(
            "slash_path={}; underscore_path={}; distinct_bytes={}; loaded_ids={},{}",
            slash_path.display(),
            underscore_path.display(),
            slash_disk != underscore_disk,
            slash_loaded.item_id,
            underscore_loaded.item_id
        ),
    );
}

// Expected before running: revocation moves persisted branch bytes into the
// revoked archive, retaining the call directory and readable record.
#[test]
fn evidence_283_revocation_preserves_branch_record_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let v2 = WorkflowV2ResultStore::new(temp.path());
    let saved = outcome("T-REV", WorkflowV2Status::Accepted, "revocable-input", true);
    v2.save_branch_outcome(CALL, &saved).unwrap();
    let current = v2.branch_outcome_path(CALL, &saved.item_id);
    let before_bytes = std::fs::read(&current).unwrap();
    let call_dir = current.parent().unwrap().to_path_buf();
    let moved = v2.delete_branch_outcomes_for_call(CALL).unwrap();
    let revoked_dir = call_dir.join("revoked");
    let revoked_paths = std::fs::read_dir(&revoked_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    let after_bytes = std::fs::read(&revoked_paths[0]).unwrap();
    assert_eq!(moved, 1);
    assert!(!current.exists());
    assert_eq!(before_bytes, after_bytes);
    assert!(call_dir.is_dir());
    let restored: WorkflowV2BranchOutcome = serde_json::from_slice(&after_bytes).unwrap();
    assert_eq!(restored, saved);
    evidence(
        "revocation-keeps-auditable-record",
        format!(
            "current_exists={}; sha256={}",
            current.exists(),
            blake3::hash(&before_bytes)
        ),
        format!(
            "moved={moved}; call_dir_exists={}; revoked_files={}; bytes_match={}; record_item={}",
            call_dir.exists(),
            revoked_paths.len(),
            before_bytes == after_bytes,
            restored.item_id
        ),
    );
}
