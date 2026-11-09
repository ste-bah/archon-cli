//! Issue #631: assertions describe the correct language-neutral behavior.
use super::*;
#[path = "../../../tests/fixtures/lang_projects/support.rs"]
mod fixture;

#[test]
fn r5_h5_declared_npx_runner_is_runnable() {
    let mut task = archon_workflow::task_universe::WorkflowV2TaskUniverseTask {
        canonical_task_id: "TASK-TS-001".into(),
        required_tools: vec!["npx".into()],
        focused_tests: vec!["npx --offline vitest run pass.test.ts".into()],
        ..Default::default()
    };
    task.freeze_focused_commands(Path::new(""), std::ffi::OsStr::new(""));
    let mut control = task.clone();
    control.focused_tests = vec!["node --test pass.test.ts".into()];
    control.required_tools.push("node".into());
    control.freeze_focused_commands(Path::new(""), std::ffi::OsStr::new(""));
    assert!(
        task_has_runnable_test(&control),
        "control node test must be runnable"
    );
    assert!(
        task_has_runnable_test(&task),
        "declared npx test runner blocked"
    );
}
