use std::path::Path;

use archon_cognitive::{
    ClassifyInput, CognitiveSurface, SituationClassifier, ToolGateInput, ToolUseGate, ToolVerdict,
};
use serde_json::json;

fn situation(text: &str) -> archon_cognitive::Situation {
    SituationClassifier.classify(ClassifyInput {
        user_text: text,
        session_id: "test-session",
        turn_number: 1,
        surface: CognitiveSurface::Tui,
    })
}

fn verdict(text: &str, tool_name: &str, tool_input: serde_json::Value) -> ToolVerdict {
    ToolUseGate.evaluate(ToolGateInput {
        situation: &situation(text),
        tool_name,
        tool_input: &tool_input,
        working_dir: Path::new("."),
    })
}

#[test]
fn greeting_suppresses_all_tools() {
    let verdict = verdict("hello", "Bash", json!({"command": "git status"}));
    assert!(matches!(verdict, ToolVerdict::Suppress { .. }));
}

#[test]
fn simple_question_does_not_block_agent_tool_choice() {
    let memory = verdict("what did we decide?", "MemoryRecall", json!({}));
    let docs = verdict("what did we decide?", "DocSearch", json!({}));
    let bash = verdict("what did we decide?", "Bash", json!({"command": "ls"}));
    assert!(memory.is_allow());
    assert!(docs.is_allow());
    assert!(bash.is_allow());
}

#[test]
fn ambiguous_followup_observes_without_blocking_tools() {
    let skill_list = verdict("yes", "Skill", json!({"action": "list"}));
    let read = verdict("yes", "Read", json!({"file_path": "README.md"}));
    let bash = verdict("yes", "Bash", json!({"command": "ls"}));
    assert!(skill_list.is_allow());
    assert!(read.is_allow());
    assert!(bash.is_allow());
}

#[test]
fn code_change_allows_file_tools() {
    let verdict = verdict("fix the bug", "Read", json!({"file_path": "src/main.rs"}));
    assert!(verdict.is_allow());
}

#[test]
fn ambiguous_subagent_request_allows_agent_tool() {
    let verdict = verdict(
        "please run a subagent",
        "Agent",
        json!({"prompt": "run the subagent"}),
    );
    assert!(verdict.is_allow());
}

#[test]
fn ci_debug_observes_bash_without_blocking_permissions() {
    let gh = verdict("ci failed", "Bash", json!({"command": "gh run view"}));
    let shell = verdict("ci failed", "Bash", json!({"command": "npm install"}));
    assert!(gh.is_allow());
    assert!(shell.is_allow());
}
