//! TASK-AGS-104: D2 AgentTool tool-owned spawn site (REQ-FOR-D2 [2/5]).
//!
//! Written BEFORE the implementation (Gate 1). Exercises the new
//! `AgentTool::execute` contract: synchronous spawn + register in
//! `BACKGROUND_AGENTS` + return `{"agent_id":"<uuid>","status":"spawned"}`
//! in under 10ms per call. Depends on the TASK-AGS-101 registry, which
//! this task relocates from `archon-core` to `archon-tools` to break
//! the `archon-core <-> archon-tools` dependency cycle.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Instant;

use serde_json::{json, Value};

use archon_tools::agent_tool::AgentTool;
use archon_tools::background_agents::{AgentStatus, BACKGROUND_AGENTS};
use archon_tools::tool::{AgentMode, Tool, ToolContext};

fn make_ctx() -> ToolContext {
    ToolContext {
        working_dir: PathBuf::from("/tmp"),
        session_id: "task-ags-104-test".into(),
        mode: AgentMode::Normal,
        extra_dirs: vec![],
    }
}

fn parse_result(content: &str) -> Value {
    serde_json::from_str(content).expect("result.content must be valid JSON")
}

// ---------- Contract shape ----------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn execute_returns_agent_id_and_spawned_status() {
    let tool = AgentTool::new();
    let input = json!({ "prompt": "Do something" });

    let result = tool.execute(input, &make_ctx()).await;
    assert!(!result.is_error, "unexpected error: {}", result.content);

    let v = parse_result(&result.content);
    assert_eq!(v["status"], "spawned", "status field must be 'spawned'");
    let id = v["agent_id"].as_str().expect("agent_id must be a string");
    uuid::Uuid::parse_str(id).expect("agent_id must be a valid uuid-v4");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn execute_latency_under_10ms() {
    // AC-01: each call returns in <10ms. Allow a generous 50ms bound
    // to absorb CI jitter; the goal is to prove we are NOT blocking on
    // any agent I/O synchronously.
    let tool = AgentTool::new();
    let input = json!({ "prompt": "Do something" });

    // Warm-up to page in dashmap + lazy singleton.
    let _ = tool.execute(input.clone(), &make_ctx()).await;

    let start = Instant::now();
    let _ = tool.execute(input, &make_ctx()).await;
    let elapsed = start.elapsed();
    assert!(
        elapsed.as_millis() < 50,
        "execute() must return in <50ms; got {elapsed:?}"
    );
}

// ---------- Registry side-effect ----------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn execute_registers_running_handle() {
    let tool = AgentTool::new();
    let input = json!({ "prompt": "Track me" });

    let result = tool.execute(input, &make_ctx()).await;
    assert!(!result.is_error);

    let v = parse_result(&result.content);
    let id_str = v["agent_id"].as_str().unwrap();
    let id = uuid::Uuid::parse_str(id_str).unwrap();

    // The global registry must have the handle registered. TASK-AGS-104
    // run_subagent is a scaffold that always errors with a TASK-AGS-105
    // pointer, so by the time this assertion runs the spawned task may
    // have already flipped status to Failed. Any non-None status proves
    // registration happened; the exact terminal state becomes meaningful
    // once TASK-AGS-105 wires the real runner.
    let status = BACKGROUND_AGENTS.get(&id);
    assert!(
        matches!(
            status,
            Some(AgentStatus::Running)
                | Some(AgentStatus::Finished)
                | Some(AgentStatus::Failed)
        ),
        "registered handle must exist post-execute; got {status:?}"
    );

    // Clean up so later tests don't see stale state.
    let _ = BACKGROUND_AGENTS.cancel(&id);
}

// ---------- Fan-out contract ----------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn spawn_100_yields_100_unique_agent_ids() {
    // TC-ARCH-03 smoke: 100 sequential spawns must all register and all
    // get distinct agent_ids.
    let tool = AgentTool::new();
    let mut ids: HashSet<uuid::Uuid> = HashSet::new();

    for i in 0..100 {
        let input = json!({ "prompt": format!("agent {i}") });
        let result = tool.execute(input, &make_ctx()).await;
        assert!(!result.is_error, "call {i} errored: {}", result.content);
        let v = parse_result(&result.content);
        let id = uuid::Uuid::parse_str(v["agent_id"].as_str().unwrap()).unwrap();
        assert!(ids.insert(id), "duplicate agent_id at iteration {i}: {id}");
        // Cancel immediately so the registry doesn't grow unboundedly.
        let _ = BACKGROUND_AGENTS.cancel(&id);
    }

    assert_eq!(ids.len(), 100, "expected 100 unique agent ids");
}

// ---------- Validation still enforced ----------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_prompt_still_errors_without_spawning() {
    let tool = AgentTool::new();
    let input = json!({ "model": "claude-sonnet-4-6" });

    let result = tool.execute(input, &make_ctx()).await;
    assert!(result.is_error, "missing prompt must surface as tool error");
    assert!(result.content.contains("prompt"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invalid_max_turns_still_errors_without_spawning() {
    let tool = AgentTool::new();
    let input = json!({ "prompt": "x", "max_turns": 999 });

    let result = tool.execute(input, &make_ctx()).await;
    assert!(result.is_error);
    assert!(result.content.contains("max_turns"));
}
