//! Issue #631: assertions describe the correct language-neutral behavior.
use super::*;
#[path = "../../../../tests/fixtures/lang_projects/support.rs"]
mod fixture;

#[test]
fn r6_p3_python_baseline_prompt_describes_actual_runner() {
    let stamp = crate::v2::verification::baseline_rule::BaselineStamp {
        base_commit: "base".into(),
        declared_commands: vec!["python3 -m unittest test_fail".into()],
        must_pass: vec!["test_fail.Failing.test_add".into()],
        ..Default::default()
    };
    let text = baseline_tests_prompt_section(&serde_json::json!({"baseline_tests":stamp}));
    assert!(!text.is_empty());
    assert!(
        !text.contains("test <name> ... FAILED"),
        "Python verifier promised a libtest-only parser"
    );
}
