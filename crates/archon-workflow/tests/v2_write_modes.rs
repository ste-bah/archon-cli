use archon_workflow::{
    WorkflowV2FileRecord, WorkflowV2Result, WorkflowV2Status, WorkflowV2WriteItem,
    WorkflowV2WriteMode, WorkflowV2WritePlanner, WorkflowV2WriteSafetyError,
    validate_changed_files,
};

#[test]
fn overlapping_targets_are_not_parallel_in_same_tree() {
    let planner = WorkflowV2WritePlanner::new("/tmp/wc");
    let plan = planner
        .plan(&[
            item("left", WorkflowV2WriteMode::Coordinated, &["src/lib.rs"]),
            item("right", WorkflowV2WriteMode::Coordinated, &["src/lib.rs"]),
        ])
        .expect("plan");

    assert_eq!(plan.waves.len(), 2);
    assert_eq!(plan.conflicts.len(), 1);
    assert_eq!(plan.conflicts[0].target, "src/lib.rs");
    assert!(!plan.conflicts[0].isolated_by_worktree);
    assert!(plan.waves.iter().all(|wave| wave.assignments.len() == 1));
}

#[test]
fn non_overlapping_targets_can_share_coordinated_wave() {
    let planner = WorkflowV2WritePlanner::new("/tmp/wc");
    let plan = planner
        .plan(&[
            item("left", WorkflowV2WriteMode::Coordinated, &["src/lib.rs"]),
            item("right", WorkflowV2WriteMode::Coordinated, &["src/main.rs"]),
        ])
        .expect("plan");

    assert_eq!(plan.waves.len(), 1);
    assert_eq!(plan.waves[0].assignments.len(), 2);
    assert!(plan.conflicts.is_empty());
}

#[test]
fn parent_child_targets_are_treated_as_overlapping() {
    let planner = WorkflowV2WritePlanner::new("/tmp/wc");
    let plan = planner
        .plan(&[
            item("left", WorkflowV2WriteMode::Coordinated, &["src"]),
            item("right", WorkflowV2WriteMode::Coordinated, &["src/lib.rs"]),
        ])
        .expect("plan");

    assert_eq!(plan.waves.len(), 2);
    assert_eq!(plan.conflicts.len(), 1);
    assert_eq!(plan.conflicts[0].target, "src");
}

#[test]
fn worktree_mode_records_paths_and_isolates_overlaps() {
    let planner = WorkflowV2WritePlanner::new("/tmp/wc");
    let plan = planner
        .plan(&[
            item("left/item", WorkflowV2WriteMode::Worktree, &["src/lib.rs"]),
            item("right:item", WorkflowV2WriteMode::Worktree, &["src/lib.rs"]),
        ])
        .expect("plan");

    assert_eq!(plan.waves.len(), 1);
    assert_eq!(plan.waves[0].assignments.len(), 2);
    assert!(
        plan.waves[0]
            .assignments
            .iter()
            .all(|assignment| assignment.worktree_path.is_some())
    );
    assert_eq!(plan.conflicts.len(), 1);
    assert!(plan.conflicts[0].isolated_by_worktree);
}

#[test]
fn worktree_paths_do_not_collide_after_sanitizing_item_ids() {
    let planner = WorkflowV2WritePlanner::new("/tmp/wc");
    let plan = planner
        .plan(&[
            item("same/id", WorkflowV2WriteMode::Worktree, &["src/lib.rs"]),
            item("same:id", WorkflowV2WriteMode::Worktree, &["src/main.rs"]),
        ])
        .expect("plan");
    let paths = plan.waves[0]
        .assignments
        .iter()
        .map(|assignment| assignment.worktree_path.as_deref().unwrap())
        .collect::<Vec<_>>();

    assert_ne!(paths[0], paths[1]);
}

#[test]
fn changed_files_outside_ownership_fail() {
    let write_item = item("left", WorkflowV2WriteMode::Serial, &["src/lib.rs"]);
    let result = accepted_result(&["src/main.rs"]);

    let err = validate_changed_files(&write_item, &result).expect_err("outside ownership");

    assert_eq!(
        err,
        WorkflowV2WriteSafetyError::ChangedFileOutsideOwnership {
            item_id: "left".to_string(),
            path: "src/main.rs".to_string(),
        }
    );
}

#[test]
fn changed_files_inside_ownership_pass() {
    let write_item = item("left", WorkflowV2WriteMode::Serial, &["src"]);
    let result = accepted_result(&["src/lib.rs", "src/main.rs"]);

    validate_changed_files(&write_item, &result).expect("inside ownership");
}

#[test]
fn accepted_write_without_changed_files_fails() {
    let write_item = item("left", WorkflowV2WriteMode::Serial, &["src/lib.rs"]);
    let result = accepted_result(&[]);

    let err = validate_changed_files(&write_item, &result).expect_err("no changed files");

    assert_eq!(
        err,
        WorkflowV2WriteSafetyError::AcceptedWriteWithoutChangedFiles("left".to_string())
    );
}

#[test]
fn patch_applied_marker_without_changed_files_is_not_task_acceptance() {
    let write_item = item(
        "implement-T050",
        WorkflowV2WriteMode::Serial,
        &["src/provider.rs"],
    );
    let mut result = accepted_result(&[]);
    result.summary = "patch applied".to_string();

    let err = validate_changed_files(&write_item, &result).expect_err("patch marker is not proof");

    assert_eq!(
        err,
        WorkflowV2WriteSafetyError::AcceptedWriteWithoutChangedFiles("implement-T050".to_string())
    );
}

#[test]
fn missing_ownership_is_rejected() {
    let planner = WorkflowV2WritePlanner::new("/tmp/wc");
    let err = planner
        .plan(&[item("left", WorkflowV2WriteMode::Serial, &[])])
        .expect_err("missing ownership");

    assert_eq!(
        err,
        WorkflowV2WriteSafetyError::MissingOwnership("left".to_string())
    );
}

#[test]
fn unsafe_targets_are_rejected() {
    let planner = WorkflowV2WritePlanner::new("/tmp/wc");
    let err = planner
        .plan(&[item(
            "left",
            WorkflowV2WriteMode::Serial,
            &["../outside.rs"],
        )])
        .expect_err("unsafe target");

    assert_eq!(
        err,
        WorkflowV2WriteSafetyError::UnsafeTarget {
            item_id: "left".to_string(),
            target: "../outside.rs".to_string(),
        }
    );
}

#[test]
fn mixed_write_modes_are_rejected() {
    let planner = WorkflowV2WritePlanner::new("/tmp/wc");
    let err = planner
        .plan(&[
            item("left", WorkflowV2WriteMode::Serial, &["src/lib.rs"]),
            item("right", WorkflowV2WriteMode::Worktree, &["src/main.rs"]),
        ])
        .expect_err("mixed modes");

    assert!(matches!(
        err,
        WorkflowV2WriteSafetyError::MixedWriteModes { item_id, .. } if item_id == "right"
    ));
}

fn item(id: &str, mode: WorkflowV2WriteMode, targets: &[&str]) -> WorkflowV2WriteItem {
    WorkflowV2WriteItem::new(
        id,
        mode,
        targets.iter().map(|target| target.to_string()).collect(),
    )
}

fn accepted_result(paths: &[&str]) -> WorkflowV2Result {
    WorkflowV2Result {
        status: WorkflowV2Status::Accepted,
        summary: "implementation changed files".to_string(),
        files_changed: paths
            .iter()
            .map(|path| WorkflowV2FileRecord::new(*path))
            .collect(),
        ..WorkflowV2Result::default()
    }
}
