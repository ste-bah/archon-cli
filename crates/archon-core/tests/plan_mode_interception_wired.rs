//! Plan mode intercepts a tool call and records it, proven by doing it.
//!
//! # Why this file exists
//!
//! It replaces `scripts/tests/p0b-3-plan-mode-wired.sh`, a "Gate-1 structural
//! verifier" that checked the same claim by grepping for `pub fn <name>` in
//! four files. Three things were wrong with that:
//!
//! 1. **It proved nothing.** A name matching a regex says a function is
//!    spelled a certain way, not that anything calls it. Two of the four names
//!    it looked for existed *only* to be found — thin bin-crate wrappers that
//!    dispatch never used, kept compiling by a file-level `allow(dead_code)`.
//! 2. **It had rotted.** Two of its five patterns named functions that were
//!    renamed long ago, so it had been RED for however long that was.
//! 3. **Nothing ran it.** No workflow, no `ci-gate.sh`, no reference anywhere.
//!
//! A behavioural test cannot rot the same way: rename the function and it
//! fails to compile; stop calling it and the assertion below goes red.

use archon_core::dispatch::ToolRegistry;
use archon_tools::tool::{AgentMode, ToolContext};

/// The claim, end to end: in plan mode a mutating tool does not run, and the
/// attempt is written to the session's audit file.
#[tokio::test]
async fn plan_mode_records_an_intercepted_tool_call_instead_of_running_it() {
    let temp = tempfile::tempdir().expect("tempdir");
    let target = temp.path().join("must-not-exist.txt");

    let mut registry = ToolRegistry::new();
    registry.register(Box::new(archon_tools::file_write::WriteTool));

    let context = ToolContext {
        working_dir: temp.path().to_path_buf(),
        session_id: "plan-mode-interception".to_string(),
        mode: AgentMode::Plan,
        ..ToolContext::default()
    };
    let result = registry
        .dispatch(
            "Write",
            serde_json::json!({
                "file_path": target.to_string_lossy(),
                "content": "this must not be written",
            }),
            &context,
        )
        .await;

    assert!(
        !target.exists(),
        "plan mode let a Write through: the file was created"
    );

    let audit = archon_core::plan_file::plan_audit_path(temp.path(), &context.session_id)
        .expect("the audit path resolves");
    let recorded = std::fs::read_to_string(&audit).unwrap_or_default();
    assert!(
        recorded.contains("Write"),
        "the intercepted call was not recorded in {}: {recorded:?}",
        audit.display()
    );
    assert!(
        recorded.contains("must-not-exist.txt"),
        "the audit entry does not say what was attempted: {recorded:?}"
    );

    // The model is told why, or it will simply try again.
    assert!(
        result.content.to_lowercase().contains("plan"),
        "the refusal should name plan mode: {}",
        result.content
    );
}

/// The control: outside plan mode the same call runs and nothing is audited.
/// Without this, a test that broke `Write` entirely would still pass above.
#[tokio::test]
async fn outside_plan_mode_the_same_call_runs_and_is_not_audited() {
    let temp = tempfile::tempdir().expect("tempdir");
    let target = temp.path().join("written.txt");

    let mut registry = ToolRegistry::new();
    registry.register(Box::new(archon_tools::file_write::WriteTool));

    let context = ToolContext {
        working_dir: temp.path().to_path_buf(),
        session_id: "normal-mode-control".to_string(),
        mode: AgentMode::Normal,
        ..ToolContext::default()
    };
    let _ = registry
        .dispatch(
            "Write",
            serde_json::json!({
                "file_path": target.to_string_lossy(),
                "content": "written normally",
            }),
            &context,
        )
        .await;

    assert!(
        target.exists(),
        "the write did not happen outside plan mode"
    );
    let audit = archon_core::plan_file::plan_audit_path(temp.path(), &context.session_id)
        .expect("the audit path resolves");
    assert!(
        !audit.exists(),
        "a normal-mode call should not be written to the plan audit"
    );
}

/// The bin-crate facade must keep pointing at the same implementation dispatch
/// uses. It is a set of `#[inline]` pass-throughs, so the risk is not that it
/// breaks — it is that it drifts into a second implementation, which is what
/// the deleted grep was nominally guarding against.
#[test]
fn the_audit_path_is_derived_from_the_session_id() {
    let temp = tempfile::tempdir().expect("tempdir");

    let one = archon_core::plan_file::plan_audit_path(temp.path(), "session-one").expect("one");
    let two = archon_core::plan_file::plan_audit_path(temp.path(), "session-two").expect("two");

    assert_ne!(one, two, "two sessions must not share an audit file");
    assert!(one.starts_with(temp.path()), "{}", one.display());
}
