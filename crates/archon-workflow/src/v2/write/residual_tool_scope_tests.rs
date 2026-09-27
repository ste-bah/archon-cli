use serde_json::{Value, json};

use super::stamp_residual_tool_scope;
use crate::WorkflowV2FanoutItem;
use crate::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};
use crate::tool_declarations::REQUIRED_TOOL_SCOPE_KEY;

fn universe() -> WorkflowV2TaskUniverse {
    WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: vec![],
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-A".into(),
            source_path: "tasks/TASK-A.md".into(),
            files_expected_to_change: vec!["src/lane.rs".into(), "scripts/view.pine".into()],
            focused_tests: vec![
                "mcp__ide__compile_check on scripts/view.pine returns no errors".into(),
                "cargo test -p lane".into(),
            ],
            acceptance_criteria: vec!["every .sql migration passes db_lint".into()],
            required_tools: vec![
                "mcp__ide__compile_check".into(),
                "db_lint".into(),
                "mcp__feed__quote".into(),
            ],
            ..Default::default()
        }],
    }
}

fn branch(residual: bool) -> WorkflowV2FanoutItem {
    let mut item = json!({"canonical_task_ids": ["TASK-A"], "task": "fix the gap",
        "required_tools": ["db_lint", "mcp__feed__quote", "mcp__ide__compile_check"],
        (REQUIRED_TOOL_SCOPE_KEY): [{"tool": "mcp__feed__quote", "files": []}]});
    if residual {
        item["residual_expansion_paths"] = json!([]);
    }
    WorkflowV2FanoutItem {
        id: "b-0".into(),
        role: "coder".into(),
        call: crate::WorkflowV2HostCall {
            id: "b".into(),
            method: crate::WorkflowV2HostMethod::Fanout,
            write_mode: None,
            options: crate::WorkflowV2HostOptions::default(),
        },
        input: json!({"item": item}),
    }
}

fn scope(branch: &WorkflowV2FanoutItem, tool: &str) -> Value {
    branch.input["item"][REQUIRED_TOOL_SCOPE_KEY]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["tool"] == tool)
        .cloned()
        .unwrap()
}

#[test]
fn a_residual_rounds_tools_are_tied_to_what_their_tasks_metadata_names() {
    let dir = tempfile::tempdir().unwrap();
    let mut branches = vec![branch(true)];
    stamp_residual_tool_scope(
        &mut branches,
        Some(&universe()),
        Some(dir.path().to_str().unwrap()),
    );
    let compile = scope(&branches[0], "mcp__ide__compile_check");
    assert_eq!(compile["files"], json!(["scripts/view.pine"]));
    assert_eq!(compile["extensions"], json!([]));
    let lint = scope(&branches[0], "db_lint");
    assert_eq!(lint["files"], json!([]));
    assert_eq!(lint["extensions"], json!([".sql"]));
    // Named by no line: tied to nothing, it carries its task's files.
    let quote = scope(&branches[0], "mcp__feed__quote");
    assert_eq!(quote["files"], json!([]));
    assert_eq!(quote["extensions"], json!([]));
    assert_eq!(
        quote["task_files"],
        json!(["scripts/view.pine", "src/lane.rs"])
    );
}

#[test]
fn no_other_branch_carries_a_scope_and_an_authored_one_is_removed() {
    let dir = tempfile::tempdir().unwrap();
    let mut branches = vec![branch(false)];
    stamp_residual_tool_scope(
        &mut branches,
        Some(&universe()),
        Some(dir.path().to_str().unwrap()),
    );
    assert!(
        branches[0].input["item"]
            .get(REQUIRED_TOOL_SCOPE_KEY)
            .is_none(),
        "{}",
        branches[0].input
    );
}
