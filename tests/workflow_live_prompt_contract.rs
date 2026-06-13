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
    assert!(
        source.contains("Do not model report-only"),
        "planner prompt must not turn report/readiness artifacts into empty implementation fanouts"
    );
    assert!(
        source.contains("external/project-artifact deliverables"),
        "planner prompt must route project artifacts through report-producing stages"
    );
    assert!(
        source.contains("Never let an empty implementation target inventory skip"),
        "planner prompt must preserve required report/readiness deliverables"
    );
}
