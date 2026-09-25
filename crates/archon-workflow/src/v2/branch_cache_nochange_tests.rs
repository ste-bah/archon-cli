//! A credited remediation write with no apply manifest replays only when its
//! record positively says it changed nothing.

use super::*;

fn no_change() -> serde_json::Value {
    serde_json::json!({
        "patch_landed": false,
        "delivery": { "kind": "no_repository_change", "repository_changed": false },
    })
}

/// A remediation write item whose tree check runs: it names a repository.
fn rooted_item(root: &std::path::Path) -> WorkflowV2FanoutItem {
    let mut item = remediation_item("TASK-B", 1, 31, "[f1]");
    item.input["item"]["target_repository_root"] = serde_json::json!(root.display().to_string());
    item
}

/// The call record `item`'s write was filed under, in `item`'s write mode:
/// the no-change proof is read against the RECORDED mode.
fn record_call(store: &WorkflowV2ResultStore, item: &WorkflowV2FanoutItem) {
    let mut call = item.call.clone();
    call.id = fanout_call_id(item);
    call.method = WorkflowV2HostMethod::Fanout;
    store
        .save_call_record(&WorkflowV2CallRecord::new(
            "run",
            call,
            1,
            "input".to_string(),
            result(WorkflowV2Status::Accepted, serde_json::json!({})),
            Vec::new(),
        ))
        .expect("record");
}

#[test]
fn a_credited_write_without_a_manifest_stands_only_on_a_recorded_no_change() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path().join("run/v2"));
    let item = rooted_item(temp.path());
    let call_id = fanout_call_id(&item);
    let artifact = |changed: serde_json::Value| {
        serde_json::json!({ "patch_landed": false, "delivery": { "kind": "project_artifact",
            "repository_changed": false, "changed_artifact_paths": changed } })
    };
    let mut serial = item.clone();
    serial.call.write_mode = Some(WorkflowV2WriteMode::Serial);
    let mut read_only = item.clone();
    read_only.call.write_mode = None;
    for (case, target, status, data, stands) in [
        (
            "recorded no change",
            &item,
            WorkflowV2Status::Accepted,
            no_change(),
            true,
        ),
        (
            "no-op, artifacts unchanged",
            &item,
            WorkflowV2Status::Noop,
            artifact(serde_json::json!([])),
            true,
        ),
        (
            "a project artifact changed",
            &item,
            WorkflowV2Status::Accepted,
            artifact(serde_json::json!(["r.md"])),
            false,
        ),
        (
            "a patch landed",
            &item,
            WorkflowV2Status::Accepted,
            serde_json::json!({ "patch_landed": true, "delivery": { "kind": "repository_patch", "repository_changed": true } }),
            false,
        ),
        (
            "unapplied patch, downgraded marker",
            &item,
            WorkflowV2Status::Accepted,
            serde_json::json!({ "patch_landed": false, "delivery": { "kind": "repository_patch", "repository_changed": true } }),
            false,
        ),
        (
            "older record, no receipt",
            &item,
            WorkflowV2Status::Accepted,
            serde_json::json!({ "patch_landed": false }),
            false,
        ),
        (
            "no landing marker",
            &item,
            WorkflowV2Status::Accepted,
            serde_json::json!({ "delivery": { "kind": "no_repository_change", "repository_changed": false } }),
            false,
        ),
        (
            "serial write, agent-written markers",
            &serial,
            WorkflowV2Status::Accepted,
            no_change(),
            false,
        ),
        (
            "history, not credit",
            &item,
            WorkflowV2Status::NeedsReview,
            serde_json::json!({ "patch_landed": true }),
            true,
        ),
        (
            "read-only branch",
            &read_only,
            WorkflowV2Status::Accepted,
            serde_json::json!({}),
            true,
        ),
    ] {
        record_call(&store, target);
        let recorded = outcome_for(target, status, None, data);
        assert_eq!(
            super::super::remediation::tree_holds_landing(&store, &call_id, &recorded, target),
            stands,
            "{case}"
        );
    }
}

/// Through the reuse split, under the branch's own id: the recorded no-change
/// answer replays, a claimed patch with no receipt is asked again.
#[test]
fn an_own_record_without_a_manifest_replays_only_when_it_changed_nothing() {
    let landed = serde_json::json!({ "patch_landed": true,
        "delivery": { "kind": "repository_patch", "repository_changed": true } });
    for (data, replays) in [(no_change(), true), (landed, false)] {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = WorkflowV2ResultStore::new(temp.path().join("run/v2"));
        let item = rooted_item(temp.path());
        let call_id = fanout_call_id(&item);
        let earlier = WorkflowV2ResultStore::new(store.root().to_path_buf());
        let mut call = item.call.clone();
        call.id = call_id.clone();
        call.method = WorkflowV2HostMethod::Fanout;
        earlier
            .save_call_record(&WorkflowV2CallRecord::new(
                "run",
                call,
                1,
                "input".to_string(),
                result(WorkflowV2Status::Accepted, serde_json::json!({})),
                Vec::new(),
            ))
            .expect("record");
        let mut data = data;
        data["branch_id"] = serde_json::json!(item.id);
        earlier
            .save_branch_outcome(
                &call_id,
                &outcome_for(&item, WorkflowV2Status::Accepted, None, data.clone()),
            )
            .expect("outcome");
        let (reused, pending) =
            split_reusable_branch_outcomes(&store, &call_id, vec![item]).expect("split");
        assert_eq!(
            (reused.len(), pending.len()),
            if replays { (1, 0) } else { (0, 1) },
            "{data}"
        );
    }
}

/// A recorded no change stands beside an idempotent (empty) manifest whose
/// snapshot of the targets later stages moved on from; beside a manifest
/// that names a landed patch, the record is contradicted and the tree rule
/// judges it.
#[test]
fn a_recorded_no_change_stands_only_beside_an_empty_manifest() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path().join("run/v2"));
    let item = rooted_item(temp.path());
    let call_id = fanout_call_id(&item);
    std::fs::create_dir_all(temp.path().join("src")).unwrap();
    std::fs::write(temp.path().join("src/a.rs"), "moved on\n").unwrap();
    record_call(&store, &item);
    let recorded = outcome_for(&item, WorkflowV2Status::Accepted, None, no_change());
    for (status, stands) in [("idempotent_noop", true), ("applied", false)] {
        let manifest = serde_json::json!({
            "schema": "archon.workflow.patch_manifest.v1", "run_id": "run", "stage_id": call_id,
            "item_id": item.id, "baseline_commit": "abc", "patch_path": "x.patch",
            "declared_target_files": ["src/a.rs"], "changed_files": [], "created_files": [],
            "deleted_files": [], "pre_hashes": {}, "post_hashes": { "src/a.rs": "then" },
            "verify_command": null, "agent_artifact_path": null, "status": { "status": status },
        });
        let path = std::path::PathBuf::from(manifest_path_for(
            store.root().parent().unwrap(),
            &call_id,
            &item.id,
        ));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert_eq!(
            super::super::remediation::tree_holds_landing(&store, &call_id, &recorded, &item),
            stands,
            "{status}"
        );
    }
}

/// A shifted write whose sibling carries no landing marker at all (a record
/// older than the marker) is not a sibling that landed nothing: with no
/// receipt it runs again. A sibling that recorded no change replays.
#[test]
fn a_sibling_without_a_landing_marker_is_not_one_that_landed_nothing() {
    for (data, replays) in [(serde_json::json!({}), false), (no_change(), true)] {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
        let sibling = remediation_item("TASK-B", 1, 31, "[f1]");
        seed_with(&store, &sibling, WorkflowV2Status::Accepted, None);
        let call_id = fanout_call_id(&sibling);
        let mut data = data;
        data["branch_id"] = serde_json::json!(sibling.id);
        let earlier = WorkflowV2ResultStore::new(store.root().to_path_buf());
        earlier
            .save_branch_outcome(
                &call_id,
                &outcome_for(&sibling, WorkflowV2Status::Accepted, None, data.clone()),
            )
            .expect("outcome");
        let now = remediation_item("TASK-B", 1, 29, "[f1]");
        let (reused, pending) =
            split_reusable_branch_outcomes(&store, &fanout_call_id(&now), vec![now])
                .expect("split");
        assert_eq!(
            (reused.len(), pending.len()),
            if replays { (1, 0) } else { (0, 1) },
            "{data}"
        );
    }
}

/// An ignored deliverable never enters the tree, and a copy at its path
/// there is the one the worktree was seeded from, not the fix's: a
/// `skipped_ignored` receipt has nothing in the tree to check.
#[test]
fn a_skipped_ignored_receipt_has_nothing_in_the_tree_to_check() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path().join("run/v2"));
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(repo.join("docs")).unwrap();
    std::fs::write(repo.join("docs/report.md"), "the seeded copy\n").unwrap();
    let item = rooted_item(&repo);
    let call_id = fanout_call_id(&item);
    let manifest = serde_json::json!({
        "schema": "archon.workflow.patch_manifest.v1", "run_id": "run", "stage_id": call_id,
        "item_id": item.id, "baseline_commit": "abc", "patch_path": "x.patch",
        "declared_target_files": ["docs/report.md"], "changed_files": [], "created_files": [],
        "deleted_files": [], "pre_hashes": {}, "post_hashes": { "docs/report.md": "fix" },
        "verify_command": null, "agent_artifact_path": null,
        "status": { "status": "skipped_ignored" },
        "skipped_ignored": { "docs/report.md": "/retained/docs/report.md" },
    });
    let path = std::path::PathBuf::from(manifest_path_for(
        store.root().parent().unwrap(),
        &call_id,
        &item.id,
    ));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let recorded = outcome_for(
        &item,
        WorkflowV2Status::Accepted,
        None,
        serde_json::json!({ "patch_landed": true }),
    );
    assert!(super::super::remediation::tree_holds_landing(
        &store, &call_id, &recorded, &item
    ));
}

/// The tree check judged the current record. A remediation write whose
/// current record is a no-change replay is reused as that record, never as
/// an older landing record the check did not judge.
#[test]
fn an_older_landing_record_never_stands_in_for_the_judged_one() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path().join("run/v2"));
    let item = rooted_item(temp.path());
    let call_id = fanout_call_id(&item);
    record_call(&store, &item);
    let earlier = WorkflowV2ResultStore::new(store.root().to_path_buf());
    let mut landed = outcome_for(
        &item,
        WorkflowV2Status::Accepted,
        None,
        serde_json::json!({ "patch_landed": true, "canonical_task_ids": ["TASK-B"],
            "delivery": { "kind": "repository_patch", "repository_changed": true } }),
    );
    landed.item_input_hash = Some("an-earlier-identity".to_string());
    earlier
        .save_branch_outcome(&call_id, &landed)
        .expect("landed");
    let mut data = no_change();
    data["canonical_task_ids"] = serde_json::json!(["TASK-B"]);
    let replay = outcome_for(&item, WorkflowV2Status::Accepted, None, data);
    earlier
        .save_branch_outcome(&call_id, &replay)
        .expect("replay");
    let (reused, pending) =
        split_reusable_branch_outcomes(&store, &call_id, vec![item]).expect("split");
    assert!(pending.is_empty());
    assert_eq!(
        reused[0].result.as_ref().unwrap().data["patch_landed"],
        serde_json::json!(false),
        "reused as the record the tree check judged"
    );
}

/// A manifest that never applied is no receipt for a credited answer: it
/// runs again. History is no credit, and what it recorded never landed.
#[test]
fn a_credited_write_whose_manifest_never_applied_runs_again() {
    for (manifest_status, status, stands) in [
        ("pending_apply", WorkflowV2Status::Accepted, false),
        ("conflicted", WorkflowV2Status::Accepted, false),
        ("pending_apply", WorkflowV2Status::NeedsReview, true),
    ] {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = WorkflowV2ResultStore::new(temp.path().join("run/v2"));
        let item = rooted_item(temp.path());
        let call_id = fanout_call_id(&item);
        let manifest = serde_json::json!({
            "schema": "archon.workflow.patch_manifest.v1", "run_id": "run", "stage_id": call_id,
            "item_id": item.id, "baseline_commit": "abc", "patch_path": "x.patch",
            "declared_target_files": [], "changed_files": [], "created_files": [],
            "deleted_files": [], "pre_hashes": {}, "post_hashes": {},
            "verify_command": null, "agent_artifact_path": null,
            "status": { "status": manifest_status },
        });
        let path = std::path::PathBuf::from(manifest_path_for(
            store.root().parent().unwrap(),
            &call_id,
            &item.id,
        ));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let data = serde_json::json!({ "patch_landed": true });
        let recorded = outcome_for(&item, status, None, data);
        assert_eq!(
            super::super::remediation::tree_holds_landing(&store, &call_id, &recorded, &item),
            stands,
            "{manifest_status} {status:?}"
        );
    }
}
