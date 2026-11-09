//! Issue #631: assertions describe the correct language-neutral behavior.
use super::*;
#[path = "../../../../../tests/fixtures/lang_projects/support.rs"]
mod fixture;

#[test]
fn r6_h2_python_is_host_runnable() {
    assert!(fixture::fixture("python").join("test_fail.py").exists());
    assert!(
        host_runnable(
            "python3 -m unittest test_fail",
            &declared("python3 -m unittest test_fail")
        ),
        "declared Python command excluded from run-base proof"
    );
}

#[test]
fn maven_focused_command_is_host_runnable() {
    assert!(host_runnable(
        "mvn -Dtest=CalculatorTest test",
        &declared("mvn -Dtest=CalculatorTest test")
    ));
}

fn declared(command: &str) -> FrozenCommands {
    let mut task = crate::task_universe::WorkflowV2TaskUniverseTask {
        focused_tests: vec![format!("`{command}`")],
        required_tools: vec![command.split_whitespace().next().unwrap().into()],
        ..Default::default()
    };
    task.freeze_focused_commands(std::path::Path::new(""), std::ffi::OsStr::new(""));
    FrozenCommands::from_universe(Some(&WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: vec![],
        tasks: vec![task],
    }))
}
