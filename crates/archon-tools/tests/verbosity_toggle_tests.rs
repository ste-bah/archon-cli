//! Tests for TASK-CLI-314: VerbosityToggle tool
//!
//! Covers: tool name (NOT BriefTool), toggle/set/status actions,
//! state transitions, schema validity, permission level.

use archon_tools::verbosity_toggle::{VerbosityState, VerbosityToggleTool};
use archon_tools::tool::{PermissionLevel, Tool, ToolContext, ToolResult};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use serde_json::json;

fn ctx() -> ToolContext {
    ToolContext {
        working_dir: PathBuf::from("/tmp"),
        session_id: "test-session".into(),
        mode: archon_tools::tool::AgentMode::Normal,
    }
}

fn make_tool() -> (VerbosityToggleTool, Arc<Mutex<VerbosityState>>) {
    let state = Arc::new(Mutex::new(VerbosityState::new(true)));
    let tool = VerbosityToggleTool::new(Arc::clone(&state));
    (tool, state)
}

// ---------------------------------------------------------------------------
// Tool identity
// ---------------------------------------------------------------------------

#[test]
fn tool_name_is_verbosity_toggle_not_brief_tool() {
    let (tool, _) = make_tool();
    assert_eq!(tool.name(), "VerbosityToggle");
    assert_ne!(tool.name(), "BriefTool", "must not be named BriefTool");
}

#[test]
fn tool_description_is_non_empty() {
    let (tool, _) = make_tool();
    assert!(!tool.description().is_empty());
}

#[test]
fn tool_input_schema_has_action_field() {
    let (tool, _) = make_tool();
    let schema = tool.input_schema();
    let props = schema["properties"].as_object().expect("schema must have properties");
    assert!(props.contains_key("action"), "schema must have 'action' property");
}

#[test]
fn tool_permission_level_is_safe() {
    let (tool, _) = make_tool();
    let level = tool.permission_level(&json!({"action": "toggle"}));
    assert_eq!(level, PermissionLevel::Safe);
}

// ---------------------------------------------------------------------------
// VerbosityState
// ---------------------------------------------------------------------------

#[test]
fn state_default_verbose_true() {
    let state = VerbosityState::new(true);
    assert!(state.is_verbose());
}

#[test]
fn state_default_verbose_false() {
    let state = VerbosityState::new(false);
    assert!(!state.is_verbose());
}

#[test]
fn state_toggle_flips_verbose() {
    let mut state = VerbosityState::new(true);
    state.toggle();
    assert!(!state.is_verbose());
    state.toggle();
    assert!(state.is_verbose());
}

#[test]
fn state_set_brief_sets_false() {
    let mut state = VerbosityState::new(true);
    state.set_verbose(false);
    assert!(!state.is_verbose());
}

#[test]
fn state_set_verbose_sets_true() {
    let mut state = VerbosityState::new(false);
    state.set_verbose(true);
    assert!(state.is_verbose());
}

#[test]
fn state_mode_string_verbose() {
    let state = VerbosityState::new(true);
    assert_eq!(state.mode_str(), "verbose");
}

#[test]
fn state_mode_string_brief() {
    let state = VerbosityState::new(false);
    assert_eq!(state.mode_str(), "brief");
}

// ---------------------------------------------------------------------------
// Tool execute — toggle
// ---------------------------------------------------------------------------

#[tokio::test]
async fn execute_toggle_from_verbose_goes_brief() {
    let (tool, state) = make_tool(); // starts verbose=true
    let result = tool.execute(json!({"action": "toggle"}), &ctx()).await;
    assert!(!result.is_error);
    assert!(!state.lock().unwrap().is_verbose());
    assert!(result.content.contains("brief"), "result should mention new mode");
}

#[tokio::test]
async fn execute_toggle_from_brief_goes_verbose() {
    let state = Arc::new(Mutex::new(VerbosityState::new(false)));
    let tool = VerbosityToggleTool::new(Arc::clone(&state));
    let result = tool.execute(json!({"action": "toggle"}), &ctx()).await;
    assert!(!result.is_error);
    assert!(state.lock().unwrap().is_verbose());
    assert!(result.content.contains("verbose"), "result should mention new mode");
}

#[tokio::test]
async fn execute_toggle_twice_returns_to_original() {
    let (tool, state) = make_tool();
    tool.execute(json!({"action": "toggle"}), &ctx()).await;
    tool.execute(json!({"action": "toggle"}), &ctx()).await;
    assert!(state.lock().unwrap().is_verbose());
}

// ---------------------------------------------------------------------------
// Tool execute — set
// ---------------------------------------------------------------------------

#[tokio::test]
async fn execute_set_brief() {
    let (tool, state) = make_tool();
    let result = tool.execute(json!({"action": "set", "mode": "brief"}), &ctx()).await;
    assert!(!result.is_error);
    assert!(!state.lock().unwrap().is_verbose());
}

#[tokio::test]
async fn execute_set_verbose() {
    let state = Arc::new(Mutex::new(VerbosityState::new(false)));
    let tool = VerbosityToggleTool::new(Arc::clone(&state));
    let result = tool.execute(json!({"action": "set", "mode": "verbose"}), &ctx()).await;
    assert!(!result.is_error);
    assert!(state.lock().unwrap().is_verbose());
}

#[tokio::test]
async fn execute_set_invalid_mode_returns_error() {
    let (tool, _) = make_tool();
    let result = tool.execute(json!({"action": "set", "mode": "blah"}), &ctx()).await;
    assert!(result.is_error, "invalid mode should return error");
}

#[tokio::test]
async fn execute_set_missing_mode_returns_error() {
    let (tool, _) = make_tool();
    let result = tool.execute(json!({"action": "set"}), &ctx()).await;
    assert!(result.is_error, "missing mode field should return error");
}

// ---------------------------------------------------------------------------
// Tool execute — status
// ---------------------------------------------------------------------------

#[tokio::test]
async fn execute_status_when_verbose() {
    let (tool, _) = make_tool();
    let result = tool.execute(json!({"action": "status"}), &ctx()).await;
    assert!(!result.is_error);
    assert!(result.content.contains("verbose"));
}

#[tokio::test]
async fn execute_status_when_brief() {
    let state = Arc::new(Mutex::new(VerbosityState::new(false)));
    let tool = VerbosityToggleTool::new(Arc::clone(&state));
    let result = tool.execute(json!({"action": "status"}), &ctx()).await;
    assert!(!result.is_error);
    assert!(result.content.contains("brief"));
}

// ---------------------------------------------------------------------------
// Tool execute — unknown action
// ---------------------------------------------------------------------------

#[tokio::test]
async fn execute_unknown_action_returns_error() {
    let (tool, _) = make_tool();
    let result = tool.execute(json!({"action": "explode"}), &ctx()).await;
    assert!(result.is_error);
}

#[tokio::test]
async fn execute_missing_action_returns_error() {
    let (tool, _) = make_tool();
    let result = tool.execute(json!({}), &ctx()).await;
    assert!(result.is_error);
}
