use archon_cognitive::{
    ClassifyInput, CognitiveSurface, SituationClassifier, ToolGateInput, ToolUseGate, ToolVerdict,
};
use serde_json::json;

#[test]
fn git_probe_in_non_repo_becomes_context_note() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let situation = SituationClassifier.classify(ClassifyInput {
        user_text: "check status",
        session_id: "test-session",
        turn_number: 1,
        surface: CognitiveSurface::Tui,
    });

    let verdict = ToolUseGate.evaluate(ToolGateInput {
        situation: &situation,
        tool_name: "Bash",
        tool_input: &json!({"command": "git status --short"}),
        working_dir: tmp.path(),
    });

    assert!(matches!(verdict, ToolVerdict::ConvertToContextNote { .. }));
    assert!(!verdict.reason().contains("exit 128"));
}
