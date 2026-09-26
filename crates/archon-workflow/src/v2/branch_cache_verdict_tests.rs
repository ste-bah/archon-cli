//! Branch reuse of a remediation verdict follows the fix it judged.

use super::*;

/// A remediation verifier branch as the host builds it: read-only, the item
/// id `{id}-check` carrying the ordinal, the verdict's completion evidence.
fn verdict_item(ordinal: u64) -> WorkflowV2FanoutItem {
    let id = format!("review-verify-task-b-1-{ordinal}");
    let call_id = format!("verification-wave-{id}");
    let mut options = WorkflowV2HostOptions::default();
    options.extra.insert(
        "remediationContract".to_string(),
        serde_json::json!({ "version": 1, "stage": "verify", "taskId": "TASK-B", "round": 1, "maxRounds": 2, "sourceReduceCallIds": ["r"] }),
    );
    let call = WorkflowV2HostCall {
        id: format!("{call_id}-0"),
        method: WorkflowV2HostMethod::Agent,
        write_mode: None,
        options,
    };
    WorkflowV2FanoutItem::read_only(
        format!("{call_id}-0"),
        "coder",
        call,
        serde_json::json!({
            "fanout_call_id": call_id,
            "item": { "item_id": format!("{id}-check"), "canonical_task_ids": ["TASK-B"], "task": "Verify [f1]" },
        }),
    )
}

/// An earlier session: fix 31 landed, verdict 32 accepted it.
fn earlier_verdict(store: &WorkflowV2ResultStore) {
    seed(
        store,
        &remediation_item("TASK-B", 1, 31, "[f1]"),
        WorkflowV2Status::Accepted,
    );
    let earlier = WorkflowV2ResultStore::new(store.root().to_path_buf());
    let verdict = verdict_item(32);
    let call_id = fanout_call_id(&verdict);
    let mut call = verdict.call.clone();
    call.id = call_id.clone();
    let mut outcome = outcome_for(
        &verdict,
        WorkflowV2Status::Accepted,
        None,
        serde_json::json!({}),
    );
    outcome.completion_evidence = vec![
        crate::v2::result_store::WorkflowV2TaskCompletionEvidence::new(
            "TASK-B",
            crate::v2::result_store::WorkflowV2TaskCompletionEvidenceKind::FocusedVerification,
            call_id.clone(),
            verdict.id.clone(),
            WorkflowV2Status::Accepted,
        ),
    ];
    // As the host saves them: the branch outcome, then the call record.
    earlier
        .save_branch_outcome(&call_id, &outcome)
        .expect("outcome");
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
}

#[test]
fn a_recorded_verdict_answers_only_the_fix_it_judged() {
    let key = crate::v2::script::resume_verdict::remediation_round_key(&verdict_item(30).call)
        .expect("key");
    for (lineage, ordinal, replays) in [
        (None, 30, false),
        (Some("review-remediate-task-b-1-27"), 30, false),
        (Some("review-remediate-task-b-1-31"), 30, true),
        (None, 32, false),
        (Some("review-remediate-task-b-1-31"), 32, true),
    ] {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
        earlier_verdict(&store);
        let replayed = lineage.map(|id| crate::v2::result_store::ReplayedFix {
            call_id: id.to_string(),
            finished_at: store.load_call_record(id).expect("load").map_or_else(
                || "2000-01-01T00:00:00+00:00".to_string(),
                |r| r.finished_at,
            ),
        });
        store.note_fix_lineage(&key, replayed);
        let item = verdict_item(ordinal);
        let (reused, pending) =
            split_reusable_branch_outcomes(&store, &fanout_call_id(&item), vec![item])
                .expect("split");
        assert_eq!(
            (reused.len(), pending.len()),
            if replays { (1, 0) } else { (0, 1) },
            "fix lineage {lineage:?}, verdict at {ordinal}"
        );
    }
}

/// Only a sibling that could answer the branch asks the audit to refresh:
/// the refresh is charged to the run's unexpected-change allowance.
#[test]
fn only_a_sibling_that_could_answer_the_branch_earns_a_drift_identity() {
    for (status, earns) in [
        (WorkflowV2Status::NeedsReview, false),
        (WorkflowV2Status::Failed, false),
        (WorkflowV2Status::Accepted, true),
    ] {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
        seed(&store, &remediation_item("TASK-B", 1, 31, "[f1]"), status);
        let mut branches = vec![remediation_item("TASK-B", 1, 29, "[f1]")];
        let call_id = fanout_call_id(&branches[0]);
        stamp_drift_identities(&mut branches, &call_id, &store).expect("stamp");
        assert_eq!(has_drift_identities(&branches[0]), earns, "{status:?}");
    }
}

/// Only true absence matches a recorded deletion; a directory or an
/// unreadable path there does not. Deletions git recorded outside the
/// declared targets count too.
#[test]
fn a_recorded_deletion_holds_only_while_nothing_is_there() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path().join("run/v2"));
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let mut item = remediation_item("TASK-B", 1, 31, "[f1]");
    item.input["item"]["target_repository_root"] = serde_json::json!(repo.display().to_string());
    let call_id = fanout_call_id(&item);
    // Issue-108: the host commits every landing; the deletions are its commit.
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .current_dir(&repo)
            .args(["-c", "user.email=a@b"])
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["init", "-q"]);
    std::fs::write(repo.join("gone.txt"), "old\n").unwrap();
    std::fs::write(repo.join("undeclared.txt"), "old\n").unwrap();
    git(&["add", "."]);
    git(&["-c", "user.name=someone", "commit", "-qm", "baseline"]);
    std::fs::remove_file(repo.join("gone.txt")).unwrap();
    std::fs::remove_file(repo.join("undeclared.txt")).unwrap();
    git(&["add", "-A"]);
    let message = format!("archon: wave 0 outputs (run run, stage {call_id})");
    git(&["-c", "user.name=archon-workflow", "commit", "-qm", &message]);
    let manifest = serde_json::json!({
        "schema": "archon.workflow.patch_manifest.v1", "run_id": "run", "stage_id": call_id, "item_id": item.id,
        "baseline_commit": "abc", "patch_path": "x.patch", "declared_target_files": ["gone.txt"],
        "changed_files": [], "created_files": [], "deleted_files": ["gone.txt", "undeclared.txt"],
        "pre_hashes": {}, "post_hashes": { "gone.txt": "deleted" }, "verify_command": null,
        "agent_artifact_path": null, "status": { "status": "applied" },
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
        serde_json::json!({}),
    );
    assert!(super::super::remediation::tree_holds_landing(
        &store, &call_id, &recorded, &item
    ));
    std::fs::create_dir_all(repo.join("gone.txt")).unwrap();
    assert!(
        !super::super::remediation::tree_holds_landing(&store, &call_id, &recorded, &item),
        "a directory is not a deletion"
    );
    std::fs::remove_dir(repo.join("gone.txt")).unwrap();
    std::fs::write(repo.join("undeclared.txt"), "back\n").unwrap();
    assert!(
        !super::super::remediation::tree_holds_landing(&store, &call_id, &recorded, &item),
        "an undeclared deletion the manifest recorded no longer holds"
    );
}

/// A verifier that ran again and was killed before its call record was
/// saved leaves a new answer beside the old record: the record's pairing
/// does not describe it, so it is asked again.
#[test]
fn a_verdict_answer_saved_after_its_record_is_asked_again() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    earlier_verdict(&store);
    let key = crate::v2::script::resume_verdict::remediation_round_key(&verdict_item(32).call)
        .expect("key");
    let fix = store
        .load_call_record("review-remediate-task-b-1-31")
        .expect("load")
        .expect("fix record");
    store.note_fix_lineage(
        &key,
        Some(crate::v2::result_store::ReplayedFix {
            call_id: fix.call.id.clone(),
            finished_at: fix.finished_at,
        }),
    );
    std::thread::sleep(std::time::Duration::from_millis(20));
    let verdict = verdict_item(32);
    let call_id = fanout_call_id(&verdict);
    let mut rerun = store
        .load_branch_outcome(&call_id, &verdict.id)
        .expect("load")
        .expect("outcome");
    rerun.result.as_mut().unwrap().summary = "a later verifier's answer".to_string();
    WorkflowV2ResultStore::new(store.root().to_path_buf())
        .save_branch_outcome(&call_id, &rerun)
        .expect("re-run outcome");
    let (reused, pending) =
        split_reusable_branch_outcomes(&store, &call_id, vec![verdict]).expect("split");
    assert_eq!((reused.len(), pending.len()), (0, 1));
}
