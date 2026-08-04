#[test]
fn live_workflow_prompt_advertises_write_capable_contract() {
    // The prompts moved to archon-workflow; this gate stays in the workspace
    // crate because `include_str!` resolves relative to the source file and
    // this path only reads from here.
    let source = include_str!("../crates/archon-workflow/src/stage_prompt.rs");
    assert!(
        source.contains("implementation.")
            || source.contains("implementation\\n")
            || source.contains("implementation,"),
        "live planner prompt must list the implementation stage kind"
    );
    assert!(
        source.contains("itemKind: \\\"implementation\\\""),
        "live planner prompt must teach V2 implementation fanout branches"
    );
    assert!(
        source.contains("target_files"),
        "implementation fanout contract must require target_files"
    );
    assert!(
        source.contains("Report-only deliverables"),
        "planner prompt must not turn report/readiness artifacts into implementation fanouts"
    );
    assert!(
        source.contains("Focused verification selection"),
        "execution prompt must keep focused verification language-agnostic"
    );
    assert!(
        source.contains("pytest path/to/test.py::test_name")
            && source.contains("./gradlew :module:test --tests")
            && source.contains("go test ./pkg -run TestName"),
        "live execution prompt must give non-Cargo focused-test examples"
    );
    assert!(
        source.contains("Shape the workflow to the task"),
        "planner prompt must prefer task-shaped workflows over fixed pipelines"
    );
    assert!(
        source.contains("Audit/review/research/planning is usually read-only"),
        "planner prompt must keep non-editing workflows read-only"
    );
    assert!(
        source.contains("Do not compute host-call ids at runtime"),
        "planner prompt must require stable host-call ids for approval/restart/resume"
    );
    assert!(
        !source.contains(
            "must be followed by focused tests, adversarial review, a remediation inventory"
        ),
        "planner prompt must not force the full remediation train onto every write workflow"
    );
}
