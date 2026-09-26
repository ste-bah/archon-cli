//! Issue-24: a landed task is never re-dispatched — the short-circuit that
//! reuses a branch as its landing record whatever the item's hash or its
//! current (replay) record say. Shares the authored-item helpers with
//! `branch_cache_tests.rs`.
use super::tests::{CALL, authored, first_item, prepared, saved_outcome, split};
use super::*;
use crate::v2::reuse_identity::reuse_identity;
use crate::write_coordinator::{ManifestStatus, PatchManifest};

/// The host's apply receipt for `item_id`, where `apply_wave` writes it.
fn write_manifest(store: &WorkflowV2ResultStore, item_id: &str, status: ManifestStatus) {
    let manifest = PatchManifest {
        schema: "archon.write_coordinator.patch_manifest.v1".into(),
        run_id: "run".into(),
        stage_id: CALL.into(),
        item_id: item_id.into(),
        baseline_commit: "abc".into(),
        patch_path: std::path::PathBuf::from("x.patch"),
        declared_target_files: vec!["src/lib.rs".into()],
        changed_files: vec!["src/lib.rs".into()],
        created_files: vec![],
        deleted_files: vec![],
        pre_hashes: Default::default(),
        post_hashes: Default::default(),
        verify_command: None,
        agent_artifact_path: None,
        status,
        skipped_ignored: Default::default(),
        materialized: Default::default(),
        materializable: Default::default(),
    };
    let run_root = store.root().parent().unwrap();
    let path =
        std::path::PathBuf::from(crate::v2::write::manifest_path_for(run_root, CALL, item_id));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
}

/// An accepted outcome whose branch reported a patch and whose task ids the
/// run recorded landed.
fn landed_outcome(item: &WorkflowV2FanoutItem, patch_landed: bool) -> WorkflowV2BranchOutcome {
    let mut outcome = saved_outcome(item);
    outcome.result.as_mut().unwrap().data = serde_json::json!({
        "branch_id": item.id,
        "canonical_task_ids": ["TASK-001"],
        "patch_landed": patch_landed,
    });
    outcome
}

/// A store holding the first item's outcome, marked as `patch_landed` says,
/// with the host's manifest at `status` when one is given.
fn store_with_landed(
    temp: &tempfile::TempDir,
    patch_landed: bool,
    status: Option<ManifestStatus>,
) -> WorkflowV2ResultStore {
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let first = first_item();
    store
        .save_branch_outcome(CALL, &landed_outcome(&first, patch_landed))
        .expect("save");
    if let Some(status) = status {
        write_manifest(&store, &first.id, status);
    }
    store
}

/// The same item re-authored: a different authored identity.
fn reauthored() -> WorkflowV2FanoutItem {
    let item = prepared(
        authored(&["src/lib.rs"], "add the parser, differently"),
        120,
    );
    assert_ne!(reuse_identity(&first_item()), reuse_identity(&item));
    item
}

/// (d) A committed task has nothing left to write: an accepted outcome whose
/// patch the host applied is reused even when the item was re-authored.
#[test]
fn a_landed_accepted_outcome_is_reused_whatever_the_hash_says() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = store_with_landed(&temp, true, Some(ManifestStatus::Applied));
    assert_eq!(split(&store, reauthored()), (1, 0));
}

/// (d) Accepted is not landed. Without the host's `applied` receipt (the
/// branch's own `patch_landed` claim is not one), or with a task the run never
/// landed, a changed hash means the item runs.
#[test]
fn an_accepted_outcome_that_did_not_land_is_not_reused_on_a_changed_hash() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = store_with_landed(&temp, false, None);
    assert_eq!(split(&store, reauthored()), (0, 1), "nothing landed");

    let temp = tempfile::tempdir().expect("tempdir");
    let store = store_with_landed(&temp, true, None);
    assert_eq!(
        split(&store, reauthored()),
        (0, 1),
        "no manifest: serial/coordinated shape"
    );

    let temp = tempfile::tempdir().expect("tempdir");
    let store = store_with_landed(&temp, true, Some(ManifestStatus::PendingApply));
    assert_eq!(
        split(&store, reauthored()),
        (0, 1),
        "manifest never applied"
    );

    let temp = tempfile::tempdir().expect("tempdir");
    let store = store_with_landed(&temp, true, Some(ManifestStatus::Applied));
    let mut wider = reauthored();
    wider.input["item"]["canonical_task_ids"] = serde_json::json!(["TASK-001", "TASK-009"]);
    assert_eq!(
        split(&store, wider),
        (0, 1),
        "claims a task the run never landed"
    );
}

/// The current record after a resume: the replay's record at `status`, saved
/// over the accepted record (which `save_branch_outcome` moved to
/// `superseded/`).
fn replay_over(
    store: &WorkflowV2ResultStore,
    item: &WorkflowV2FanoutItem,
    status: WorkflowV2Status,
) {
    // A later write, so the archived accepted record is older on disk.
    std::thread::sleep(std::time::Duration::from_millis(20));
    let mut replay = saved_outcome(&reauthored());
    replay.status = status;
    let result = replay.result.as_mut().unwrap();
    result.status = status;
    result.data = serde_json::json!({"branch_id": item.id, "canonical_task_ids": ["TASK-001"],
        "patch_landed": false});
    store
        .save_branch_outcome(CALL, &replay)
        .expect("save replay");
    let superseded = store.load_superseded_branch_outcomes();
    assert_eq!(superseded.len(), 1, "the accepted record was archived");
}

fn noop_replay_over(store: &WorkflowV2ResultStore, item: &WorkflowV2FanoutItem) {
    replay_over(store, item, WorkflowV2Status::Noop);
}

/// The reused outcome is the record that landed the branch, and the current
/// file now holds it.
fn assert_reused_as_landing_record(store: &WorkflowV2ResultStore, item: WorkflowV2FanoutItem) {
    let (reused, pending) = split_reusable_branch_outcomes(store, CALL, vec![item]).expect("split");
    assert!(pending.is_empty());
    assert_eq!(reused.len(), 1);
    assert_eq!(reused[0].status, WorkflowV2Status::Accepted);
    assert_eq!(
        reused[0].result.as_ref().unwrap().data["patch_landed"],
        serde_json::json!(true)
    );
    let current = store
        .load_branch_outcome(CALL, &reused[0].item_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        current, reused[0],
        "the current record is the landing record"
    );
}

/// Live after several resumes: the landed task's current record is the
/// replay's `Noop`, the accepted record is under `superseded/`, and the
/// original applied manifest is still there. It must reuse.
#[test]
fn a_landed_task_reuses_through_its_noop_replay_record() {
    let first = first_item();
    let temp = tempfile::tempdir().expect("tempdir");
    let store = store_with_landed(&temp, true, Some(ManifestStatus::Applied));
    noop_replay_over(&store, &first);
    assert_reused_as_landing_record(&store, reauthored());

    // The superseded accepted record alone is a receipt too.
    let temp = tempfile::tempdir().expect("tempdir");
    let store = store_with_landed(&temp, true, None);
    noop_replay_over(&store, &first);
    assert_reused_as_landing_record(&store, reauthored());
}

/// Live on (agents-4, agents-4-0): the replay of a landed task blew the line
/// cap and ended `needs_review`, so the current record refused reuse and the
/// committed task was dispatched yet again. The landing record — superseded,
/// accepted, `patch_landed` — is what the branch is reused as, and it is
/// re-saved as the current record.
#[test]
fn a_landed_task_reuses_its_landing_record_whatever_a_replay_said() {
    let first = first_item();
    let temp = tempfile::tempdir().expect("tempdir");
    let store = store_with_landed(&temp, true, Some(ManifestStatus::Applied));
    replay_over(&store, &first, WorkflowV2Status::NeedsReview);
    assert_reused_as_landing_record(&store, reauthored());
    // The replay's record was kept under superseded/, not lost.
    let archived = store.load_superseded_branch_outcomes();
    assert!(
        archived
            .iter()
            .any(|o| o.status == WorkflowV2Status::NeedsReview),
        "{archived:#?}"
    );
}

/// A needs-review current record with no landed evidence anywhere is just a
/// failed replay: it runs.
#[test]
fn a_needs_review_record_without_landed_evidence_is_not_reused() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let mut review = saved_outcome(&first_item());
    review.status = WorkflowV2Status::NeedsReview;
    let result = review.result.as_mut().unwrap();
    result.status = WorkflowV2Status::NeedsReview;
    result.data = serde_json::json!({"canonical_task_ids": ["TASK-001"], "patch_landed": false});
    store.save_branch_outcome(CALL, &review).expect("save");
    assert!(store.load_superseded_branch_outcomes().is_empty());
    assert_eq!(split(&store, reauthored()), (0, 1));
    // Even with the hash unchanged: needs_review is never reused.
    assert_eq!(split(&store, first_item()), (0, 1));
}

/// A no-op current record with no landed evidence anywhere is just a no-op
/// whose hash moved: it runs.
#[test]
fn a_noop_record_without_landed_evidence_is_not_reused_on_a_changed_hash() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let mut noop = saved_outcome(&first_item());
    noop.status = WorkflowV2Status::Noop;
    let result = noop.result.as_mut().unwrap();
    result.status = WorkflowV2Status::Noop;
    result.data = serde_json::json!({"canonical_task_ids": ["TASK-001"]});
    store.save_branch_outcome(CALL, &noop).expect("save");
    assert!(store.load_superseded_branch_outcomes().is_empty());
    assert_eq!(split(&store, reauthored()), (0, 1));
}

/// Issue-26: an accepted outcome whose only deliverable the repository ignores.
/// The host's manifest is `skipped_ignored` — nothing to commit — and the
/// audit has reclaimed the path. It reuses on the hash
/// match alone; with the path still in the audit's question it was
/// re-dispatched on every resume (live: TASK-DL-001, six re-runs).
#[test]
fn an_accepted_outcome_whose_only_deliverable_is_ignored_reuses_on_its_hash() {
    use crate::repository_audit::budget::{AuditBudget, AuditPolicy, Limit};
    use crate::repository_audit::runtime::{AuditState, STATE_PATH, Snapshot};
    let item = prepared(authored(&["docs/x.md"], "write the gap audit"), 120);
    let audit_state = |reclaimed: bool| {
        let mut state = AuditState {
            schema_version: 1,
            generation: 1,
            ledger: Default::default(),
            snapshot: None,
            attempts: 1,
            last_error: None,
            final_receipt: None,
            operator_controls: vec![],
            policy_provenance: None,
            declared_paths: ["src/lib.rs".to_string()].into_iter().collect(),
            budget: AuditBudget::new(AuditPolicy {
                attempt_timeout_secs: Limit::Unlimited,
                total_time_secs: Limit::Unlimited,
                unexpected_change_refreshes: Limit::Unlimited,
            }),
        };
        state.snapshot = Some(Snapshot {
            identity: "one".into(),
            root: "/nowhere".into(),
            paths: vec![],
        });
        state
            .ledger
            .accept(
                crate::repository_audit::AuditContract {
                    schema_version: 1,
                    snapshot: "one".into(),
                    declared_paths: vec!["src/lib.rs".into()],
                },
                serde_json::from_value(
                    serde_json::json!({"schema_version": 1, "snapshot": "one", "records": [{
                "declared_path": "src/lib.rs", "verdict": "exists_as_declared", "equivalents": [],
                "required_action": "none", "reason": "present"}]}),
                )
                .unwrap(),
            )
            .unwrap();
        if reclaimed {
            state.ledger.ignored_paths.insert("docs/x.md".into());
        }
        state
    };
    for (reclaimed, expected) in [(false, (0, 1)), (true, (1, 0))] {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
        let mut outcome = saved_outcome(&item);
        outcome.result.as_mut().unwrap().data = serde_json::json!({"branch_id": item.id,
            "canonical_task_ids": ["TASK-001"], "patch_landed": false,
            "skipped_ignored": {"docs/x.md": "artifacts/ignored-deliverables/x/docs/x.md"}});
        store.save_branch_outcome(CALL, &outcome).expect("save");
        write_manifest(&store, &item.id, ManifestStatus::SkippedIgnored);
        let path = temp.path().join(STATE_PATH);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, serde_json::to_vec(&audit_state(reclaimed)).unwrap()).unwrap();
        assert_eq!(
            split(&store, item.clone()),
            expected,
            "reclaimed={reclaimed}"
        );
    }
}
