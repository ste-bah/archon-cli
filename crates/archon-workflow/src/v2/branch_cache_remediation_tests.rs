//! Branch reuse for finished review maps and for review remediation that
//! drifted to another prelude ordinal or was superseded by a later round.

use super::*;

use crate::v2::result::{WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2Result};
use crate::v2::result_store::WorkflowV2CallRecord;
use crate::v2::reuse_identity::stamp_reuse_input_hash;
use crate::v2::scheduler::BranchFailureKind;
use crate::v2::write::manifest_path_for;
use crate::write_coordinator::{ManifestStatus, PatchManifest};
use crate::{WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions, WorkflowV2WriteMode};

fn result(status: WorkflowV2Status, data: serde_json::Value) -> WorkflowV2Result {
    let mut result = WorkflowV2Result::accepted("branch answered");
    result.status = status;
    result.data = data;
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Implementation,
        "branch recorded concrete evidence",
    ));
    result
}

fn outcome_for(
    item: &WorkflowV2FanoutItem,
    status: WorkflowV2Status,
    failure_kind: Option<BranchFailureKind>,
    data: serde_json::Value,
) -> WorkflowV2BranchOutcome {
    WorkflowV2BranchOutcome {
        item_id: item.id.clone(),
        role: item.role.clone(),
        status,
        result: Some(result(status, data)),
        error: None,
        failure_kind,
        item_input_hash: Some(reuse_identity(item)),
        completion_evidence: Vec::new(),
    }
}

fn review_map_item(index: usize) -> WorkflowV2FanoutItem {
    let mut options = WorkflowV2HostOptions::default();
    options.extra.insert(
        "reviewContract".to_string(),
        serde_json::json!({ "version": 1, "kind": "uncovered_requirements", "stage": "map", "findingsPath": "data.findings" }),
    );
    let call = WorkflowV2HostCall {
        id: format!("coverage-audit-map-{index}"),
        method: WorkflowV2HostMethod::Agent,
        write_mode: None,
        options,
    };
    WorkflowV2FanoutItem::read_only(
        format!("coverage-audit-map-{index}"),
        "critic",
        call,
        serde_json::json!({
            "fanout_call_id": "coverage-audit-map",
            "item": { "item_id": format!("review-task-{index}"), "canonical_task_ids": [format!("TASK-{index}")] },
        }),
    )
}

#[test]
fn a_finished_review_map_branch_with_findings_is_reused() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let finished = review_map_item(12);
    let broken = review_map_item(13);
    let findings = serde_json::json!({ "findings": [{ "id": "gap", "claim": "REQ-7 uncovered" }] });
    store
        .save_branch_outcome(
            "coverage-audit-map",
            &outcome_for(
                &finished,
                WorkflowV2Status::NeedsReview,
                Some(BranchFailureKind::Semantic),
                findings.clone(),
            ),
        )
        .expect("save");
    store
        .save_branch_outcome(
            "coverage-audit-map",
            &outcome_for(
                &broken,
                WorkflowV2Status::NeedsReview,
                Some(BranchFailureKind::Contract),
                findings,
            ),
        )
        .expect("save");
    let (reused, pending) = split_reusable_branch_outcomes(
        &store,
        "coverage-audit-map",
        vec![finished.clone(), broken.clone()],
    )
    .expect("split");
    assert_eq!(reused.len(), 1, "the finished review is replayed");
    assert_eq!(reused[0].item_id, finished.id);
    assert_eq!(reused[0].status, WorkflowV2Status::NeedsReview);
    assert_eq!(pending.len(), 1, "a contract failure is asked again");
    assert_eq!(pending[0].id, broken.id);
}

/// A review remediation write branch as `call_data::source` builds it for the
/// prelude's `agent({write: true})`, with the authored identity stamped.
fn remediation_item(task: &str, round: u64, ordinal: u64, findings: &str) -> WorkflowV2FanoutItem {
    let call_id = format!(
        "review-remediate-{}-{round}-{ordinal}",
        task.to_ascii_lowercase()
    );
    let mut options = WorkflowV2HostOptions::default();
    options.target_files_from_item = true;
    options.extra.insert(
        "remediationContract".to_string(),
        serde_json::json!({ "version": 1, "stage": "remediate", "taskId": task, "round": round, "maxRounds": 2, "sourceReduceCallIds": ["r"] }),
    );
    let call = WorkflowV2HostCall {
        id: format!("{call_id}-0"),
        method: WorkflowV2HostMethod::Implementation,
        write_mode: Some(WorkflowV2WriteMode::Worktree),
        options,
    };
    let mut branches = vec![WorkflowV2FanoutItem::read_only(
        format!("{call_id}-0"),
        "coder",
        call,
        serde_json::json!({
            "fanout_call_id": call_id,
            "fanout_item_id": call_id,
            "item": { "item_id": call_id, "canonical_task_ids": [task], "task": format!("Fix {findings}"), "target_files": ["src/a.rs"] },
        }),
    )];
    stamp_reuse_input_hash(&mut branches);
    branches.remove(0)
}

fn fanout_call_id(item: &WorkflowV2FanoutItem) -> String {
    item.input["fanout_call_id"]
        .as_str()
        .expect("call id")
        .to_string()
}

/// Record `item`'s call and branch as an EARLIER session did: through a
/// store instance of its own, so the session under test holds none of it.
fn seed(store: &WorkflowV2ResultStore, item: &WorkflowV2FanoutItem, status: WorkflowV2Status) {
    seed_with(store, item, status, Some(ManifestStatus::Applied));
}

fn seed_with(
    store: &WorkflowV2ResultStore,
    item: &WorkflowV2FanoutItem,
    status: WorkflowV2Status,
    manifest: Option<ManifestStatus>,
) {
    let earlier = WorkflowV2ResultStore::new(store.root().to_path_buf());
    let call_id = fanout_call_id(item);
    let mut call = item.call.clone();
    call.id = call_id.clone();
    call.method = WorkflowV2HostMethod::Fanout;
    let record_result = result(status, serde_json::json!({}));
    earlier
        .save_call_record(&WorkflowV2CallRecord::new(
            "run",
            call,
            1,
            "input".to_string(),
            record_result,
            Vec::new(),
        ))
        .expect("record");
    let data = serde_json::json!({ "patch_landed": true, "branch_id": item.id });
    let failure = (status != WorkflowV2Status::Accepted).then_some(BranchFailureKind::Contract);
    earlier
        .save_branch_outcome(&call_id, &outcome_for(item, status, failure, data))
        .expect("outcome");
    if let Some(status) = manifest {
        write_manifest(store, &call_id, &item.id, status);
    }
}

/// The host's apply receipt for a branch, where `apply_wave` writes it.
fn write_manifest(
    store: &WorkflowV2ResultStore,
    call_id: &str,
    item_id: &str,
    status: ManifestStatus,
) {
    let manifest = PatchManifest {
        schema: "archon.write_coordinator.patch_manifest.v1".into(),
        run_id: "run".into(),
        stage_id: call_id.into(),
        item_id: item_id.into(),
        baseline_commit: "abc".into(),
        patch_path: std::path::PathBuf::from("x.patch"),
        declared_target_files: vec!["src/a.rs".into()],
        changed_files: vec!["src/a.rs".into()],
        created_files: vec![],
        deleted_files: vec![],
        pre_hashes: Default::default(),
        post_hashes: Default::default(),
        verify_command: None,
        agent_artifact_path: None,
        status,
        skipped_ignored: Default::default(),
    };
    let run_root = store.root().parent().expect("run root");
    let path = std::path::PathBuf::from(manifest_path_for(run_root, call_id, item_id));
    std::fs::create_dir_all(path.parent().expect("dir")).expect("mkdir");
    std::fs::write(path, serde_json::to_vec(&manifest).expect("json")).expect("write");
}

#[test]
fn a_shifted_write_whose_patch_never_went_through_apply_runs_again() {
    for manifest in [None, Some(ManifestStatus::PendingApply)] {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
        seed_with(
            &store,
            &remediation_item("TASK-B", 1, 31, "[f1]"),
            WorkflowV2Status::Accepted,
            manifest.clone(),
        );
        let now = remediation_item("TASK-B", 1, 29, "[f1]");
        let (reused, pending) =
            split_reusable_branch_outcomes(&store, &fanout_call_id(&now), vec![now])
                .expect("split");
        assert!(
            reused.is_empty(),
            "{manifest:?}: a captured patch is not a landed one"
        );
        assert_eq!(pending.len(), 1);
    }
}

#[test]
fn a_sibling_already_replayed_this_session_answers_no_second_call() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    seed(
        &store,
        &remediation_item("TASK-B", 1, 31, "[f1]"),
        WorkflowV2Status::Accepted,
    );
    let first = remediation_item("TASK-B", 1, 29, "[f1]");
    let (reused, _) = split_reusable_branch_outcomes(&store, &fanout_call_id(&first), vec![first])
        .expect("split");
    assert_eq!(reused.len(), 1);
    let second = remediation_item("TASK-B", 1, 33, "[f1]");
    let (reused, pending) =
        split_reusable_branch_outcomes(&store, &fanout_call_id(&second), vec![second])
            .expect("split");
    assert!(
        reused.is_empty(),
        "each earlier record answers at most one call"
    );
    assert_eq!(pending.len(), 1);
}

#[test]
fn a_remediation_write_under_a_shifted_ordinal_reuses_the_same_work() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    seed(
        &store,
        &remediation_item("TASK-B", 1, 31, "[f1]"),
        WorkflowV2Status::Accepted,
    );
    let now = remediation_item("TASK-B", 1, 29, "[f1]");
    let (reused, pending) =
        split_reusable_branch_outcomes(&store, &fanout_call_id(&now), vec![now.clone()])
            .expect("split");
    assert!(
        pending.is_empty(),
        "the drifted write must not be dispatched again"
    );
    assert_eq!(reused[0].item_id, now.id);
    assert_eq!(
        reused[0].item_input_hash.as_deref(),
        Some(reuse_identity(&now).as_str())
    );
    assert_eq!(
        reused[0].result.as_ref().unwrap().data["branch_id"],
        serde_json::json!(now.id),
        "the refiled outcome names this branch, not its sibling"
    );
    let saved = store
        .load_branch_outcome(&fanout_call_id(&now), &now.id)
        .expect("load")
        .expect("refiled under the new call");
    assert_eq!(saved, reused[0]);

    let changed = remediation_item("TASK-B", 1, 29, "[f2]");
    let (reused, pending) =
        split_reusable_branch_outcomes(&store, &fanout_call_id(&changed), vec![changed])
            .expect("split");
    assert!(reused.is_empty(), "different findings are different work");
    assert_eq!(pending.len(), 1);
    let other_round = remediation_item("TASK-B", 2, 29, "[f1]");
    let (reused, _) =
        split_reusable_branch_outcomes(&store, &fanout_call_id(&other_round), vec![other_round])
            .expect("split");
    assert!(reused.is_empty(), "round 2 is never answered from round 1");
}

#[test]
fn a_rejected_round_is_replayed_as_history_only_once_a_later_round_exists() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let round_one = remediation_item("TASK-E", 1, 37, "[f1]");
    seed(&store, &round_one, WorkflowV2Status::NeedsReview);
    let (reused, pending) = split_reusable_branch_outcomes(
        &store,
        &fanout_call_id(&round_one),
        vec![round_one.clone()],
    )
    .expect("split");
    assert!(
        reused.is_empty(),
        "the last round may have been in flight: ask again"
    );
    assert_eq!(pending.len(), 1);

    seed(
        &store,
        &remediation_item("TASK-E", 2, 39, "[f1]"),
        WorkflowV2Status::Accepted,
    );
    let (reused, pending) = split_reusable_branch_outcomes(
        &store,
        &fanout_call_id(&round_one),
        vec![round_one.clone()],
    )
    .expect("split");
    assert!(
        pending.is_empty(),
        "a superseded round is the answer the script acted on"
    );
    assert_eq!(reused[0].status, WorkflowV2Status::NeedsReview);

    let drifted = remediation_item("TASK-E", 1, 35, "[f1]");
    let (reused, pending) =
        split_reusable_branch_outcomes(&store, &fanout_call_id(&drifted), vec![drifted.clone()])
            .expect("split");
    assert!(pending.is_empty());
    assert_eq!(reused[0].item_id, drifted.id);
    assert_eq!(reused[0].status, WorkflowV2Status::NeedsReview);
}
