//! Batch O (I1): an item's focused tests are its tasks' declared commands.

use super::*;
use crate::task_universe::WorkflowV2TaskUniverseTask;
use crate::v2::{WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions};

fn universe() -> WorkflowV2TaskUniverse {
    let task = |id: &str, tests: &[&str]| WorkflowV2TaskUniverseTask {
        canonical_task_id: id.into(),
        focused_tests: tests.iter().map(|t| t.to_string()).collect(),
        ..Default::default()
    };
    WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: Vec::new(),
        tasks: vec![
            task(
                "TASK-A",
                &["cargo test -p a store", "cargo test -p a gates"],
            ),
            task("TASK-B", &[]),
        ],
    }
}

fn branch(item: serde_json::Value) -> crate::WorkflowV2FanoutItem {
    let call = WorkflowV2HostCall {
        id: "c".into(),
        method: WorkflowV2HostMethod::Implementation,
        write_mode: None,
        options: WorkflowV2HostOptions::default(),
    };
    crate::WorkflowV2FanoutItem::read_only(
        "c-0",
        "coder",
        call,
        serde_json::json!({ "item": item }),
    )
}

fn stamped(item: serde_json::Value) -> serde_json::Value {
    let mut branches = vec![branch(item)];
    stamp_declared_focused_tests(&mut branches, Some(&universe()));
    branches.remove(0).input["item"].clone()
}

#[test]
fn a_dropped_or_invented_command_is_replaced_by_the_declared_set() {
    let item = stamped(serde_json::json!({
        "canonical_task_ids": ["TASK-A"], "work_type": "implementation",
        "focused_verification": ["cargo test -p a store", "cargo test -p a invented"],
    }));
    assert_eq!(
        item["focused_verification"],
        serde_json::json!(["cargo test -p a store", "cargo test -p a gates"])
    );
    assert_eq!(
        item[AUTHORED_FOCUSED_TESTS_KEY],
        serde_json::json!(["cargo test -p a store", "cargo test -p a invented"])
    );
    // A write item the script gave no command at all runs the declared ones.
    let empty = stamped(serde_json::json!({
        "canonical_task_ids": ["TASK-A"], "work_type": "implementation", "focusedTests": [],
    }));
    assert_eq!(empty["focused_verification"].as_array().unwrap().len(), 2);
    assert!(empty.get("focusedTests").is_none());
}

#[test]
fn the_same_set_in_another_order_and_host_planned_or_goal_items_are_left_alone() {
    for item in [
        serde_json::json!({"canonical_task_ids": ["TASK-A"], "work_type": "implementation",
            "focused_verification": ["cargo test -p a gates", "cargo test -p a store"]}),
        serde_json::json!({"canonical_task_ids": ["TASK-A"], "focused_verification": [],
            "verification_requirements": ["prove it"]}),
        serde_json::json!({"canonical_task_ids": ["TASK-A"], "work_type": "implementation",
            "residual_expansion_paths": ["src/x.rs"], "focused_verification": ["cargo test x"]}),
        serde_json::json!({"canonical_task_ids": ["TASK-A"], "work_type": "implementation",
            "escalation_owner_task_ids": ["TASK-B"], "focused_verification": ["cargo test x"]}),
        serde_json::json!({"canonical_task_ids": ["TASK-B"], "work_type": "implementation",
            "focused_verification": ["cargo test b"]}),
    ] {
        assert_eq!(stamped(item.clone()), item);
    }
}
