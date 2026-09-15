use super::*;

use crate::v2::result::{WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2Result};
use crate::v2::scheduler::BranchFailureKind;
use crate::{WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions};

fn accepted_result() -> WorkflowV2Result {
    let mut result = WorkflowV2Result::accepted("branch produced the declared change");
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Implementation,
        "branch recorded concrete implementation evidence",
    ));
    result
}

fn item(call_id: &str, item_id: &str) -> WorkflowV2FanoutItem {
    let call = WorkflowV2HostCall {
        id: format!("{call_id}-{item_id}"),
        method: WorkflowV2HostMethod::Implementation,
        write_mode: None,
        options: WorkflowV2HostOptions::default(),
    };
    WorkflowV2FanoutItem::read_only(
        format!("{call_id}-{item_id}"),
        "coder",
        call,
        serde_json::json!({
            "fanout_call_id": call_id,
            "fanout_item_id": item_id,
            "item": {"id": item_id, "target_files": ["src/lib.rs"]},
        }),
    )
}

fn accepted_outcome(item: &WorkflowV2FanoutItem) -> WorkflowV2BranchOutcome {
    WorkflowV2BranchOutcome {
        item_id: item.id.clone(),
        role: item.role.clone(),
        status: WorkflowV2Status::Accepted,
        result: Some(accepted_result()),
        error: None,
        failure_kind: None,
        item_input_hash: Some(item.input_hash()),
        completion_evidence: Vec::new(),
    }
}

#[test]
fn accepted_outcome_is_reused_for_the_same_call_and_item() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let item = item("remediation-wave-1", "task-a");
    store
        .save_branch_outcome("remediation-wave-1", &accepted_outcome(&item))
        .expect("save outcome");

    // Not a wave call id for the completion-evidence rule, so the accepted
    // outcome is reusable on its own terms.
    let (reused, pending) =
        split_reusable_branch_outcomes(&store, "restartable-fanout", vec![item.clone()])
            .expect("split");

    assert!(reused.is_empty(), "different call id must not match");
    assert_eq!(pending.len(), 1);

    store
        .save_branch_outcome("restartable-fanout", &accepted_outcome(&item))
        .expect("save outcome");
    let (reused, pending) =
        split_reusable_branch_outcomes(&store, "restartable-fanout", vec![item]).expect("split");

    assert_eq!(reused.len(), 1);
    assert!(pending.is_empty());
}

/// The retry wave carries only work that did NOT resolve, so an outcome stored
/// under the previous attempt's call id must stay invisible to it — even when
/// the retry re-derives a byte-identical item payload. See this module's header
/// for why widening the key would let a review loop credit a fix that did not
/// stick.
#[test]
fn cross_attempt_reuse_is_refused() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let first = item("remediation-wave-1", "task-a");
    let mut outcome = accepted_outcome(&first);
    outcome.completion_evidence = Vec::new();
    store
        .save_branch_outcome("remediation-wave-1", &outcome)
        .expect("save outcome");

    // Same item payload, retried under the call id the lifecycle driver mints
    // for the follow-up wave.
    let retried = item("remediation-wave-1-1", "task-a");

    let (reused, pending) =
        split_reusable_branch_outcomes(&store, "remediation-wave-1-1", vec![retried])
            .expect("split");

    assert!(reused.is_empty());
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, "remediation-wave-1-1-task-a");
}

/// A stored outcome that claims `accepted` while carrying a failure kind is
/// self-contradicting. Reuse must take the pessimistic reading: crediting failed
/// work as done is unrecoverable, re-running it is merely expensive.
#[test]
fn accepted_outcome_carrying_a_failure_kind_is_not_reused() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let item = item("restartable-fanout", "task-a");
    let mut outcome = accepted_outcome(&item);
    outcome.failure_kind = Some(BranchFailureKind::Safety);
    store
        .save_branch_outcome("restartable-fanout", &outcome)
        .expect("save outcome");

    let (reused, pending) =
        split_reusable_branch_outcomes(&store, "restartable-fanout", vec![item]).expect("split");

    assert!(reused.is_empty());
    assert_eq!(pending.len(), 1);
}

#[test]
fn wave_call_ids_require_completion_evidence_before_reuse() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let item = item("implementation-wave-1", "task-a");
    store
        .save_branch_outcome("implementation-wave-1", &accepted_outcome(&item))
        .expect("save outcome");

    let (reused, pending) =
        split_reusable_branch_outcomes(&store, "implementation-wave-1", vec![item]).expect("split");

    assert!(reused.is_empty());
    assert_eq!(pending.len(), 1);
}

#[test]
fn repository_audit_missing_state_cannot_credit_a_cached_write_branch() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let mut branch = item("cached-write", "one");
    branch.call.write_mode = Some(crate::WorkflowV2WriteMode::Worktree);
    branch.call.options.target_files = vec!["new.txt".into()];
    store.save_branch_outcome("cached-write", &accepted_outcome(&branch)).unwrap();
    let audit_dir = store.root().join("repository-audit");
    std::fs::create_dir_all(&audit_dir).unwrap();
    std::fs::write(audit_dir.join("required.json"), "{\"schema_version\":1}").unwrap();
    let result = split_reusable_branch_outcomes(&store, "cached-write", vec![branch]);
    assert!(result.is_err(), "mandatory audit state disappeared but cached write was credited");
}

// ---------------------------------------------------------------------------
// Issue-24: reuse keys on the item as authored, and a landed task is never
// re-dispatched.
// ---------------------------------------------------------------------------

use crate::v2::reuse_identity::{
    REUSE_INPUT_HASH_KEY, recorded_hash_matches, reuse_identity, reuse_input_hash,
    stamp_reuse_input_hash,
};
use crate::write_coordinator::{ManifestStatus, PatchManifest};

const CALL: &str = "restartable-fanout";

/// A write branch as the script authors it, before the host stamps anything.
fn authored(target_files: &[&str], prompt: &str) -> WorkflowV2FanoutItem {
    let mut item = item(CALL, "task-a");
    item.call.write_mode = Some(crate::WorkflowV2WriteMode::Worktree);
    item.call.options.target_files = target_files.iter().map(|t| (*t).to_string()).collect();
    item.input = serde_json::json!({
        "fanout_call_id": CALL,
        "fanout_item_id": "task-a",
        "item": {
            "id": "task-a",
            "canonical_task_ids": ["TASK-001"],
            "target_files": target_files,
            "prompt": prompt,
        },
    });
    item
}

/// What `write::run_write_capable_v2_fanout` does to a branch before it asks
/// for reuse: the identity stamp first, then the volatile stamps.
fn prepared(mut branch: WorkflowV2FanoutItem, current_lines: u32) -> WorkflowV2FanoutItem {
    stamp_reuse_input_hash(std::slice::from_mut(&mut branch));
    let object = branch.input.as_object_mut().unwrap();
    object.insert(
        "_workflow_project_artifact_policy".into(),
        serde_json::json!({"version": 1, "project_root": "/p", "artifact_roots": ["docs"]}),
    );
    let item = object["item"].as_object_mut().unwrap();
    item.insert("target_repository_root".into(), "/repo".into());
    item.insert(
        "required_tools".into(),
        serde_json::json!(["mcp__x__fetch"]),
    );
    item.insert("max_source_file_lines".into(), 500.into());
    item.insert(
        "target_file_budgets".into(),
        serde_json::json!([{"path": "src/lib.rs", "current_lines": current_lines,
            "max_lines": 500, "lines_remaining": 500 - current_lines}]),
    );
    branch
}

/// The item of the first dispatch, as prepared.
fn first_item() -> WorkflowV2FanoutItem {
    prepared(authored(&["src/lib.rs"], "add the parser"), 120)
}

/// An accepted outcome saved the way the write layer saves it now.
fn saved_outcome(item: &WorkflowV2FanoutItem) -> WorkflowV2BranchOutcome {
    let mut outcome = accepted_outcome(item);
    outcome.item_input_hash = Some(reuse_identity(item));
    outcome
}

fn split(store: &WorkflowV2ResultStore, item: WorkflowV2FanoutItem) -> (usize, usize) {
    let (reused, pending) = split_reusable_branch_outcomes(store, CALL, vec![item]).expect("split");
    (reused.len(), pending.len())
}

/// (a) The live defect: a later wave grew `src/lib.rs`, so on resume the
/// earlier item's line budget differed and its hash moved. The root, the tool
/// binding and a rewritten (discovered) scope are the other stamps that move
/// without the authored item moving.
#[test]
fn stamps_that_move_with_the_tree_do_not_change_the_reuse_identity() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let first = first_item();
    store
        .save_branch_outcome(CALL, &saved_outcome(&first))
        .expect("save");

    let mut resumed = prepared(authored(&["src/lib.rs"], "add the parser"), 480);
    resumed.input["item"]["target_files"] = serde_json::json!(["src/lib.rs", "src/parser.rs"]);
    assert_ne!(first.input, resumed.input, "the stamped inputs differ");
    assert_ne!(
        first.input_hash(),
        resumed.input_hash(),
        "the old full hash would refuse"
    );

    assert_eq!(split(&store, resumed), (1, 0));
}

/// The projection alone, for a caller that never stamped the identity: every
/// volatile key is removed, nothing authored is.
#[test]
fn reuse_input_hash_strips_exactly_the_host_stamps() {
    let bare = authored(&["src/lib.rs"], "add the parser");
    let mut stamped = prepared(bare.clone(), 33);
    assert_eq!(
        reuse_input_hash(&bare.input),
        reuse_input_hash(&stamped.input)
    );
    assert_eq!(reuse_identity(&bare), reuse_identity(&stamped));
    // Idempotent: stamping again changes nothing.
    let before = stamped.input.clone();
    stamp_reuse_input_hash(std::slice::from_mut(&mut stamped));
    assert_eq!(before, stamped.input);
    // The identity is what was stamped, and it is the projection.
    assert_eq!(
        stamped.input[REUSE_INPUT_HASH_KEY].as_str(),
        Some(reuse_input_hash(&bare.input).as_str())
    );
}

/// (b) Anything the script authored is identity: a different task, prompt or
/// declared scope is a different item and must run.
#[test]
fn authored_differences_are_not_reused() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    store
        .save_branch_outcome(CALL, &saved_outcome(&first_item()))
        .expect("save");

    let other_prompt = prepared(authored(&["src/lib.rs"], "add the lexer"), 120);
    assert_eq!(split(&store, other_prompt), (0, 1), "prompt");

    let other_targets = prepared(authored(&["src/lexer.rs"], "add the parser"), 120);
    assert_eq!(
        split(&store, other_targets),
        (0, 1),
        "declared target_files"
    );

    let mut other_task = authored(&["src/lib.rs"], "add the parser");
    other_task.input["item"]["canonical_task_ids"] = serde_json::json!(["TASK-002"]);
    assert_eq!(split(&store, prepared(other_task, 120)), (0, 1), "task id");
}

/// (c) An outcome stored by the previous binary carries the hash of the whole
/// stamped input. With the stamps unchanged it still reuses; with a stamp
/// changed it matches neither hash and runs — the pre-Issue-24 behaviour for
/// pre-Issue-24 records, never anything worse.
#[test]
fn legacy_full_input_hash_still_reuses_when_the_stamps_are_identical() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    // The old binary: stamps applied, no identity stamp, full hash stored.
    let mut legacy = first_item();
    legacy
        .input
        .as_object_mut()
        .unwrap()
        .remove(REUSE_INPUT_HASH_KEY);
    let mut outcome = accepted_outcome(&legacy);
    outcome.item_input_hash = Some(legacy.input_hash());
    store.save_branch_outcome(CALL, &outcome).expect("save");

    let same_stamps = first_item();
    assert!(recorded_hash_matches(&legacy.input_hash(), &same_stamps));
    assert_eq!(split(&store, same_stamps), (1, 0));

    let moved_stamp = prepared(authored(&["src/lib.rs"], "add the parser"), 121);
    assert!(!recorded_hash_matches(&legacy.input_hash(), &moved_stamp));
    assert_eq!(split(&store, moved_stamp), (0, 1));
}

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
