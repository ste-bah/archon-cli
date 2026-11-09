//! Issue #631: assertions describe the correct language-neutral behavior.
use super::*;
#[path = "../../../tests/fixtures/lang_projects/support.rs"]
mod fixture;

#[test]
fn r7_2_go_stage_gets_relevant_command_policy() {
    assert!(fixture::fixture("go").join("go.mod").exists());
    let guidance = command_execution_guidance();
    assert!(
        !guidance.contains("Cargo command policy"),
        "unconditional Rust policy injected into Go command guidance"
    );
}
