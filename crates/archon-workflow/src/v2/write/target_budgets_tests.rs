use super::*;
use crate::{
    WorkflowV2FanoutItem, WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions,
    WorkflowV2WriteMode,
};

fn branch(targets: &[&str]) -> WorkflowV2FanoutItem {
    WorkflowV2FanoutItem {
        id: "branch-1".to_string(),
        role: "coder".to_string(),
        call: WorkflowV2HostCall {
            id: "impl".to_string(),
            method: WorkflowV2HostMethod::Fanout,
            write_mode: Some(WorkflowV2WriteMode::Coordinated),
            options: WorkflowV2HostOptions::default(),
        },
        input: serde_json::json!({ "item": { "target_files": targets } }),
    }
}

fn budgets(branch: &WorkflowV2FanoutItem) -> Vec<serde_json::Value> {
    branch.input["item"]["target_file_budgets"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// The live case: a 495-line file with a cap of 500 has five lines left, and
/// the branch that grew it by 17 lost all 21 of its edits.
#[test]
fn an_existing_file_reports_what_is_left() {
    let dir = tempfile::tempdir().expect("root");
    let path = dir.path().join("src/big.rs");
    std::fs::create_dir_all(path.parent().unwrap()).expect("mkdir");
    std::fs::write(&path, "x\n".repeat(495)).expect("write");

    let mut branches = vec![branch(&["src/big.rs"])];
    stamp_target_file_budgets(&mut branches, dir.path().to_str(), 500);

    let b = &budgets(&branches[0])[0];
    assert_eq!(b["current_lines"], 495);
    assert_eq!(b["lines_remaining"], 5);
    assert_eq!(b["max_lines"], 500);
}

/// A file that does not exist yet has the whole cap.
#[test]
fn a_new_file_has_the_whole_cap() {
    let dir = tempfile::tempdir().expect("root");
    let mut branches = vec![branch(&["src/new.rs"])];
    stamp_target_file_budgets(&mut branches, dir.path().to_str(), 500);

    let b = &budgets(&branches[0])[0];
    assert_eq!(b["current_lines"], 0);
    assert_eq!(b["lines_remaining"], 500);
}

/// Already at the cap: nothing may be added, reported as zero rather than
/// underflowing.
#[test]
fn a_file_at_the_cap_reports_zero_remaining() {
    let dir = tempfile::tempdir().expect("root");
    let path = dir.path().join("full.rs");
    std::fs::write(&path, "x\n".repeat(640)).expect("write");

    let mut branches = vec![branch(&["full.rs"])];
    stamp_target_file_budgets(&mut branches, dir.path().to_str(), 500);

    assert_eq!(budgets(&branches[0])[0]["lines_remaining"], 0);
}

/// A branch declaring no targets is left untouched.
#[test]
fn a_branch_without_targets_is_not_stamped() {
    let dir = tempfile::tempdir().expect("root");
    let mut branches = vec![branch(&[])];
    stamp_target_file_budgets(&mut branches, dir.path().to_str(), 500);
    assert!(
        branches[0].input["item"]
            .get("target_file_budgets")
            .is_none()
    );
}

/// No repository root: nothing can be measured, so nothing is claimed.
#[test]
fn without_a_repository_root_nothing_is_stamped() {
    let mut branches = vec![branch(&["src/thing.rs"])];
    stamp_target_file_budgets(&mut branches, None, 500);
    assert!(
        branches[0].input["item"]
            .get("target_file_budgets")
            .is_none()
    );
}
