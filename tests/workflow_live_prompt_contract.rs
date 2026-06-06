#[test]
fn live_workflow_prompt_advertises_write_capable_contract() {
    let source = include_str!("../src/command/workflow_live_prompt.rs");
    assert!(
        source.contains("implementation.")
            || source.contains("implementation\\n")
            || source.contains("implementation,"),
        "live planner prompt must list the implementation stage kind"
    );
    assert!(
        source.contains("item_kind: implementation"),
        "live planner prompt must teach implementation fanout branches"
    );
    assert!(
        source.contains("target_files"),
        "implementation fanout contract must require target_files"
    );
}
