use super::*;
use crate::WorkflowV2HostMethod;
#[test]
fn all_noop_wave_retains_noop_instead_of_claiming_implementation() {
    let call = WorkflowV2HostCall {
        id: "noop-wave".into(),
        method: WorkflowV2HostMethod::Fanout,
        write_mode: Some(WorkflowV2WriteMode::Worktree),
        options: Default::default(),
    };
    let mut branch = WorkflowV2Result::noop("already satisfied");
    branch.data = serde_json::json!({"item_id":"one","canonical_task_ids":["TASK-001"]});
    branch.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Inspection,
        "existing output checked",
    ));
    branch.task_coverage=serde_json::from_value(serde_json::json!([{"task_id":"TASK-001","status":"noop","summary":"existing file checked","evidence":[{"kind":"inspection","summary":"existing output checked"}]}])).unwrap();
    let planner = WorkflowV2WritePlanner::new(PathBuf::from("/tmp/noop-plan"));
    let plan = planner
        .plan(&[WorkflowV2WriteItem::artifact_only(
            "one",
            WorkflowV2WriteMode::Worktree,
        )])
        .unwrap();
    let result = result_from_write_fanout(&call, vec![branch], &plan, 0, None);
    assert_eq!(result.status, WorkflowV2Status::Noop, "{result:#?}");
}

/// Issue-69: the empty-patch gate asks which declared artifacts changed
/// BEFORE `stamp` runs, so the answer must be right at any time after capture.
#[test]
fn changed_paths_reports_modified_and_created_artifacts_only() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    std::fs::write(root.join("modified.json"), "v1").unwrap();
    std::fs::write(root.join("unchanged.json"), "same").unwrap();
    let declared = |name: &str| (name.to_string(), root.join(name));
    let delivery = super::delivery::ArtifactDelivery::from_paths(vec![
        declared("modified.json"),
        declared("unchanged.json"),
        declared("created.json"),
        declared("placeholder.json"),
        declared("never.json"),
    ]);
    assert!(delivery.changed_paths().is_empty(), "nothing ran yet");
    std::fs::write(root.join("modified.json"), "v2").unwrap();
    std::fs::write(root.join("created.json"), "{\"cells\":30}").unwrap();
    std::fs::write(root.join("placeholder.json"), "").unwrap();
    let mut changed = delivery.changed_paths();
    changed.sort();
    assert_eq!(
        changed,
        vec!["created.json".to_string(), "modified.json".to_string()],
        "a modified file and a newly created non-empty file are deliveries; an \
         unchanged file, an empty placeholder and a still-missing file are not"
    );
}

#[test]
fn changed_paths_agrees_with_stamped_receipt_for_modified_artifact() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("latest.json");
    std::fs::write(&path, "before").unwrap();
    let delivery = super::delivery::ArtifactDelivery::from_paths(vec![(
        ".archon/coverage/latest.json".to_string(),
        path.clone(),
    )]);
    std::fs::write(&path, "after").unwrap();
    assert_eq!(
        delivery.changed_paths(),
        vec![".archon/coverage/latest.json".to_string()]
    );
    let mut result = WorkflowV2Result::accepted("regenerated coverage");
    delivery.stamp(&mut result, false);
    assert_eq!(result.status, WorkflowV2Status::Accepted, "{result:#?}");
    assert_eq!(result.data["delivery"]["kind"], "project_artifact");
    assert_eq!(result.data["delivery"]["repository_changed"], false);
    assert_eq!(
        result.data["delivery"]["changed_artifact_paths"],
        serde_json::json!([".archon/coverage/latest.json"])
    );
}
