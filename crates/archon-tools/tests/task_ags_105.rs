//! TASK-AGS-105: SubagentExecutor trait contract tests.
//!
//! The `TaskCreateTool` half of this suite lives in
//! `task_ags_105_task_create.rs`; both share `common::RecordingExecutor`.

mod common;

use std::sync::Arc;

use serde_json::json;

use archon_tools::agent_tool::{AgentTool, SubagentRequest};
use archon_tools::subagent_executor::{
    SubagentClassification, SubagentExecutor, get_subagent_executor,
};
use archon_tools::tool::Tool;

use common::{RecordingExecutor, make_ctx, recording_executor};
use std::sync::atomic::Ordering;

// Test 1: trait remains object-safe. Historical test name retained so the
// baseline records the original contract identity.
// ---------------------------------------------------------------------------
#[test]
fn trait_is_object_safe_with_five_methods() {
    // Compile-time check: the trait can be used as `dyn SubagentExecutor`.
    fn _requires_object_safe(_x: Arc<dyn SubagentExecutor>) {}
    // Semantic check: a boxed trait object satisfies Send+Sync+'static so
    // it can live inside the global OnceLock.
    let e: Arc<dyn SubagentExecutor> = Arc::new(RecordingExecutor::new());
    _requires_object_safe(e);
}

// Test 2: install_subagent_executor + get_subagent_executor round-trip.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn install_then_get_round_trips() {
    recording_executor();
    let exec = get_subagent_executor();
    assert!(
        exec.is_some(),
        "executor must be retrievable after install_subagent_executor"
    );
}

// ---------------------------------------------------------------------------
// Test 3: classify routes run_in_background:true to ExplicitBackground.
// ---------------------------------------------------------------------------
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn classify_routes_background_flag() {
    recording_executor();
    let exec = get_subagent_executor().expect("installed");
    let bg_req = SubagentRequest {
        prompt: "bg".into(),
        model: None,
        allowed_tools: vec![],
        max_turns: 10,
        timeout_secs: 300,
        subagent_type: None,
        run_in_background: true,
        cwd: None,
        isolation: None,
        provider_env: None,
    };
    let fg_req = SubagentRequest {
        run_in_background: false,
        ..bg_req.clone()
    };
    assert!(matches!(
        exec.classify(&bg_req),
        SubagentClassification::ExplicitBackground
    ));
    assert!(matches!(
        exec.classify(&fg_req),
        SubagentClassification::Foreground
    ));
}

// ---------------------------------------------------------------------------
// Test 4: AgentTool::execute with run_in_background: true returns a spawn
// marker JSON (NOT the real text) — preserves the TASK-AGS-104 contract for
// the background path.
// ---------------------------------------------------------------------------
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn agent_tool_background_returns_spawn_marker() {
    recording_executor();
    let tool = AgentTool::new();
    let input = json!({ "prompt": "do bg", "run_in_background": true });
    let result = tool.execute(input, &make_ctx()).await;
    assert!(!result.is_error, "unexpected error: {}", result.content);
    let v: serde_json::Value =
        serde_json::from_str(&result.content).expect("background path must return JSON");
    assert_eq!(v["status"], "spawned");
    assert!(v["agent_id"].is_string());
}

// ---------------------------------------------------------------------------
// Test 5: AgentTool::execute with run_in_background:false (default) calls
// run_to_completion on the installed executor. We prove this by checking
// the recording executor's `ran` flag AFTER execute returns — for the
// foreground path, execute must NOT return before run_to_completion was
// invoked at least once on the executor.
// ---------------------------------------------------------------------------
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn agent_tool_foreground_invokes_run_to_completion() {
    let exec = recording_executor();
    exec.ran.store(false, Ordering::SeqCst);
    exec.inner_completed.store(false, Ordering::SeqCst);
    let tool = AgentTool::new();
    let input = json!({ "prompt": "do fg", "run_in_background": false });

    let result = tool.execute(input, &make_ctx()).await;

    assert!(!result.is_error, "unexpected error: {}", result.content);
    assert_eq!(result.content, "recorded");
    assert!(exec.ran.load(Ordering::SeqCst));
    assert!(exec.inner_completed.load(Ordering::SeqCst));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn auto_background_ms_zero_disables_timer_arm() {
    recording_executor();
    let exec = get_subagent_executor().expect("installed");
    assert_eq!(
        exec.auto_background_ms(),
        0,
        "RecordingExecutor::auto_background_ms returns 0"
    );
    let tool = AgentTool::new();
    let input = json!({ "prompt": "fast fg", "run_in_background": false });
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tool.execute(input, &make_ctx()),
    )
    .await
    .expect("foreground execute must not hang with auto_bg=0");
    assert!(!result.is_error, "unexpected error: {}", result.content);
}
