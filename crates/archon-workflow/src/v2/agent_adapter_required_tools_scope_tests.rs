// A host-planned residual round owes only the required tools its claim or
// its changed and granted files need (`scoped_required_tools`); every other
// branch owes every declared tool.

use super::*;
use crate::WorkflowV2FileRecord;
use crate::tool_declarations::REQUIRED_TOOL_SCOPE_KEY;
use serde_json::json;

fn residual_input(claim: &str, granted: &[&str]) -> serde_json::Value {
    json!({"item": {
        "canonical_task_ids": ["TASK-A"],
        "task": claim,
        "residual_expansion_paths": granted,
        "required_tools": ["mcp__ide__compile_check", "db_lint", "mcp__feed__quote"],
        (REQUIRED_TOOL_SCOPE_KEY): [
            {"tool": "mcp__ide__compile_check", "files": ["scripts/view.pine"], "extensions": [], "task_files": []},
            {"tool": "db_lint", "files": [], "extensions": [".sql"], "task_files": []},
            {"tool": "mcp__feed__quote", "files": [], "extensions": [], "task_files": ["src/feed"]},
        ],
    }})
}

fn accepted_changing(paths: &[&str]) -> WorkflowV2Result {
    WorkflowV2Result {
        status: WorkflowV2Status::Accepted,
        summary: "fixed".into(),
        files_changed: paths
            .iter()
            .map(|path| WorkflowV2FileRecord {
                path: path.to_string(),
                purpose: None,
            })
            .collect(),
        ..WorkflowV2Result::default()
    }
}

#[test]
fn a_residual_no_op_owes_no_tool_its_gaps_and_files_never_involve() {
    let input = residual_input("Host round r: the store drops a version", &[]);
    assert!(!request_declares_required_tools(&input));
    assert!(unexercised_required_tools(&input, &accepted_changing(&[])).is_empty());
}

#[test]
fn a_residual_round_owes_a_tool_its_claim_names_or_its_files_need() {
    // Named by the claim.
    let input = residual_input("Host round r: db_lint fails on the schema", &[]);
    assert!(request_declares_required_tools(&input));
    assert_eq!(
        unexercised_required_tools(&input, &accepted_changing(&[])),
        ["db_lint"]
    );
    // A changed file the metadata ties to a tool, by path or by kind.
    let input = residual_input("Host round r: fix it", &[]);
    let mut owed = unexercised_required_tools(
        &input,
        &accepted_changing(&["scripts/view.pine", "migrations/001.sql"]),
    );
    owed.sort();
    assert_eq!(owed, ["db_lint", "mcp__ide__compile_check"]);
    // A granted file needs its tool even before it changes.
    let input = residual_input("Host round r: fix it", &["scripts/view.pine"]);
    assert_eq!(
        unexercised_required_tools(&input, &accepted_changing(&[])),
        ["mcp__ide__compile_check"]
    );
    // A tool tied to no file: owed once the round changes a file its task
    // declares.
    let input = residual_input("Host round r: fix it", &[]);
    assert_eq!(
        unexercised_required_tools(&input, &accepted_changing(&["src/feed/lane.rs"])),
        ["mcp__feed__quote"]
    );
}

#[test]
fn a_branch_without_the_hosts_scope_owes_every_declared_tool() {
    let mut input = residual_input("Host round r: fix it", &[]);
    input["item"]
        .as_object_mut()
        .unwrap()
        .remove(REQUIRED_TOOL_SCOPE_KEY);
    assert!(request_declares_required_tools(&input));
    assert_eq!(
        unexercised_required_tools(&input, &accepted_changing(&[])).len(),
        3
    );
    // A tool the scope does not list is owed too.
    let mut input = residual_input("Host round r: fix it", &[]);
    input["item"][REQUIRED_TOOL_SCOPE_KEY] = json!([]);
    assert_eq!(
        unexercised_required_tools(&input, &accepted_changing(&[])).len(),
        3
    );
}
