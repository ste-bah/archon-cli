//! The focused-test completion signal: a write session whose declared tests
//! have all passed is told to submit, and bounded after that.
use archon_tools::workflow_read_guard::{FocusedTestPlan, WorkflowReadGuard};
use serde_json::json;

const DECLARED: [&str; 2] = ["cargo nextest run -p x --test a", "test -s docs/y.md"];

fn guard(grace: u32) -> WorkflowReadGuard {
    WorkflowReadGuard::new(40, 20, false, false).with_focused_tests(FocusedTestPlan::new(
        DECLARED.iter().map(|c| c.to_string()).collect(),
        grace,
    ))
}

fn bash(command: &str) -> serde_json::Value {
    json!({"command": command})
}

#[test]
fn completion_message_arrives_once_after_every_declared_test_exits_zero() {
    let guard = guard(15);
    // A failing run of a declared test does not count.
    assert!(guard.before_tool("Bash", &bash(DECLARED[0])).is_none());
    guard.after_tool("Bash", &bash(DECLARED[0]), false, "exit 1");
    assert!(guard.completion_message().is_none());
    // Whitespace differences and a longer chain both match the declared text.
    assert!(guard.before_tool("Bash", &bash("cd crates/x &&  cargo   nextest run -p x --test a")).is_none());
    guard.after_tool("Bash", &bash("cd crates/x &&  cargo   nextest run -p x --test a"), true, "exit 0");
    assert!(guard.completion_message().is_none(), "one of two is not completion");
    assert!(guard.before_tool("Bash", &bash(DECLARED[1])).is_none());
    guard.after_tool("Bash", &bash(DECLARED[1]), true, "exit 0");
    let message = guard.completion_message().expect("completion after both passed");
    assert!(message.contains("All declared focused tests have passed in this session (2 of 2 at tool call 3)"), "{message}");
    assert!(message.contains("Return the result envelope now."));
    assert!(message.contains("residual_gaps, not fixed"));
    assert!(guard.completion_message().is_none(), "handed out exactly once");
}

#[test]
fn past_the_grace_allowance_inspection_and_tests_are_refused_but_writes_admitted() {
    let guard = guard(3);
    for command in DECLARED {
        guard.before_tool("Bash", &bash(command));
        guard.after_tool("Bash", &bash(command), true, "exit 0");
    }
    assert!(guard.completion_message().is_some());
    // Three further calls of any shape are admitted.
    assert!(guard.before_tool("Read", &json!({"file_path": "a.rs"})).is_none());
    assert!(guard.before_tool("Bash", &bash("cargo check -p x")).is_none());
    assert!(guard.before_tool("Grep", &json!({"pattern": "fn"})).is_none());
    // The fourth inspection is refused with the instruction repeated.
    let refused = guard.before_tool("Bash", &bash("cat src/lib.rs")).expect("inspection refused");
    assert!(refused.contains("All declared focused tests have passed in this session (2 of 2"), "{refused}");
    assert!(refused.contains("Return the result envelope now."));
    assert!(refused.contains("3 were allowed"), "{refused}");
    assert!(guard.before_tool("Bash", &bash("cargo test -p x")).is_some(), "a test run is refused");
    assert!(guard.before_tool("Read", &json!({"file_path": "a.rs"})).is_some());
    // A last edit is never refused.
    assert!(guard.before_tool("Write", &json!({"file_path": "a.rs", "content": "x"})).is_none());
    assert!(guard.before_tool("Edit", &json!({"file_path": "a.rs"})).is_none());
}

#[test]
fn a_session_with_no_declared_tests_never_receives_the_message() {
    let guard = WorkflowReadGuard::new(40, 20, false, false)
        .with_focused_tests(FocusedTestPlan::new(vec!["  ".to_string()], 15));
    for command in DECLARED {
        guard.before_tool("Bash", &bash(command));
        guard.after_tool("Bash", &bash(command), true, "exit 0");
    }
    assert!(guard.completion_message().is_none());
    for _ in 0..20 {
        assert!(guard.before_tool("Bash", &bash("cargo test -p x")).is_none());
    }
    let plain = WorkflowReadGuard::new(40, 20, false, false);
    plain.after_tool("Bash", &bash(DECLARED[0]), true, "exit 0");
    assert!(plain.completion_message().is_none());
}

#[test]
fn the_plan_reaches_a_guard_built_inside_the_scope() {
    let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
    let guard = runtime.block_on(archon_tools::workflow_read_guard::scope_focused_tests(
        FocusedTestPlan::new(vec![DECLARED[1].to_string()], 15),
        async { WorkflowReadGuard::new(40, 20, false, false) },
    ));
    guard.before_tool("Bash", &bash(DECLARED[1]));
    guard.after_tool("Bash", &bash(DECLARED[1]), true, "exit 0");
    assert!(guard.completion_message().is_some());
}

#[test]
fn generated_config_carries_the_submit_grace_default() {
    let default: crate::config::GeneratedWorkflowConfig = serde_json::from_value(json!({})).unwrap();
    assert_eq!(default.submit_grace_calls, 15);
    assert_eq!(default.timeout_retry_budget_secs, 1_800);
    assert_eq!(default.resume_memory_calls, 12);
}
