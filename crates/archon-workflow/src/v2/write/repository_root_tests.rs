use super::*;
use crate::{
    WorkflowV2FanoutItem, WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions,
    WorkflowV2WriteMode,
};

fn branch(item: serde_json::Value) -> WorkflowV2FanoutItem {
    WorkflowV2FanoutItem {
        id: "branch-1".to_string(),
        role: "coder".to_string(),
        call: WorkflowV2HostCall {
            id: "impl".to_string(),
            method: WorkflowV2HostMethod::Fanout,
            write_mode: Some(WorkflowV2WriteMode::Coordinated),
            options: WorkflowV2HostOptions::default(),
        },
        input: serde_json::json!({ "item": item }),
    }
}

fn root_of(branch: &WorkflowV2FanoutItem) -> Option<String> {
    branch.input["item"]["target_repository_root"]
        .as_str()
        .map(str::to_string)
}

/// The live failure: an implementation item carried `project_artifact_root`
/// and a worktree path but no repository root, so the agent prefixed the
/// artifact root and hunted for `<project>/crates/...` files that cannot exist.
#[test]
fn an_implementation_item_is_told_where_the_repository_is() {
    let mut branches = [branch(serde_json::json!({ "target_files": ["src/a.rs"] }))];

    stamp_target_repository_root(&mut branches, Some("/repo/archon-cli"));

    assert_eq!(root_of(&branches[0]).as_deref(), Some("/repo/archon-cli"));
}

/// The stamp must land inside `item`, which is the object the prompt renders.
/// A key at the input's top level is carried in the record and never seen.
#[test]
fn the_root_lands_inside_the_item_object() {
    let mut branches = [branch(serde_json::json!({ "target_files": [] }))];

    stamp_target_repository_root(&mut branches, Some("/repo/archon-cli"));

    assert!(branches[0].input["item"]["target_repository_root"].is_string());
    assert!(branches[0].input["target_repository_root"].is_null());
}

/// A branch that already carries a root was given it deliberately; replacing it
/// would silently move where that agent resolves every path.
#[test]
fn an_existing_root_is_not_overwritten() {
    let mut branches = [branch(serde_json::json!({
        "target_repository_root": "/deliberate/root",
    }))];

    stamp_target_repository_root(&mut branches, Some("/repo/archon-cli"));

    assert_eq!(root_of(&branches[0]).as_deref(), Some("/deliberate/root"));
}

/// A blank recorded root is not a deliberate choice, so it is filled in.
#[test]
fn a_blank_existing_root_is_replaced() {
    let mut branches = [branch(
        serde_json::json!({ "target_repository_root": "  " }),
    )];

    stamp_target_repository_root(&mut branches, Some("/repo/archon-cli"));

    assert_eq!(root_of(&branches[0]).as_deref(), Some("/repo/archon-cli"));
}

/// With no root to give, the item is left exactly as it was rather than
/// gaining an empty value an agent would try to resolve against.
#[test]
fn nothing_is_stamped_without_a_root() {
    for root in [None, Some(""), Some("   ")] {
        let mut branches = [branch(serde_json::json!({ "target_files": [] }))];

        stamp_target_repository_root(&mut branches, root);

        assert!(
            root_of(&branches[0]).is_none(),
            "no root should be stamped for {root:?}"
        );
    }
}

/// Every branch in the fanout needs it, not just the first.
#[test]
fn all_branches_are_stamped() {
    let mut branches = [
        branch(serde_json::json!({ "target_files": ["a.rs"] })),
        branch(serde_json::json!({ "target_files": ["b.rs"] })),
        branch(serde_json::json!({ "target_files": ["c.rs"] })),
    ];

    stamp_target_repository_root(&mut branches, Some("/repo/archon-cli"));

    for item in &branches {
        assert_eq!(root_of(item).as_deref(), Some("/repo/archon-cli"));
    }
}

/// An input without an `item` object is skipped rather than panicking.
#[test]
fn a_branch_without_an_item_is_skipped() {
    let mut branches = [WorkflowV2FanoutItem {
        id: "branch-1".to_string(),
        role: "coder".to_string(),
        call: WorkflowV2HostCall {
            id: "impl".to_string(),
            method: WorkflowV2HostMethod::Fanout,
            write_mode: Some(WorkflowV2WriteMode::Coordinated),
            options: WorkflowV2HostOptions::default(),
        },
        input: serde_json::json!({ "not_an_item": true }),
    }];

    stamp_target_repository_root(&mut branches, Some("/repo/archon-cli"));

    assert!(branches[0].input["not_an_item"].as_bool().unwrap_or(false));
}
