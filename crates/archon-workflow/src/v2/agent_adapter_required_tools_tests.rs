// A task that declares required_tools (e.g. project MCP tools) must actually
// exercise them this run; inspecting stale artifacts and declaring a no-op is
// not acceptance. The write branch input carries the task's declared
// required_tools (stamped from the authoritative task universe), and the no-op
// guard reads it here.

use super::*;
use crate::{
    WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2HostCall, WorkflowV2HostMethod,
    WorkflowV2HostOptions, WorkflowV2TaskCoverage, WorkflowV2TaskCoverageStatus,
    WorkflowV2WriteMode,
};

fn write_request_with_input(input: serde_json::Value) -> WorkflowV2AgentRequest {
    WorkflowV2AgentRequest {
        call: WorkflowV2HostCall {
            id: "implement-task-tdl-120-1-0".to_string(),
            method: WorkflowV2HostMethod::Implementation,
            write_mode: Some(WorkflowV2WriteMode::Coordinated),
            options: WorkflowV2HostOptions::default(),
        },
        role: "coder".to_string(),
        task: "Implement TASK-TDL-120".to_string(),
        constraints: Vec::new(),
        input,
        repository_root: Some("/repo".to_string()),
        project_artifacts: Default::default(),
        target_files: vec!["crates/archon-trading/src/data_lake.rs".to_string()],
        target_ownership_scopes: Vec::new(),
    }
}

fn noop_result() -> WorkflowV2Result {
    let mut result = WorkflowV2Result::default();
    result.status = WorkflowV2Status::Noop;
    result.summary = "already complete: artifacts present".to_string();
    result.task_coverage.push(WorkflowV2TaskCoverage {
        task_id: "TASK-TDL-120".to_string(),
        status: WorkflowV2TaskCoverageStatus::Noop,
        summary: "existing pine artifacts inspected".to_string(),
        evidence: vec![WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Inspection,
            "read existing compile-report.json",
        )],
    });
    result
}

#[test]
fn noop_is_rejected_when_the_task_declares_required_tools() {
    // The item payload rides inside the branch input, exactly as the write
    // path stamps it.
    let input = serde_json::json!({
        "fanout_item_id": "implement-task-tdl-120-1-0",
        "item": {
            "canonical_task_ids": ["TASK-TDL-120"],
            "required_tools": ["pine_compile", "pine_get_errors"]
        }
    });
    let error = WorkflowV2AgentAdapter::new()
        .parse_agent_output(
            &write_request_with_input(input),
            &serde_json::to_string(&noop_result()).expect("serialize"),
        )
        .expect_err("noop must be rejected when required_tools are declared");
    assert!(
        matches!(
            error,
            WorkflowV2AgentError::ImplementationNoopWithDeclaredRequiredTools
        ),
        "{error}"
    );
}

#[test]
fn noop_is_accepted_when_no_required_tools_are_declared() {
    let input = serde_json::json!({
        "fanout_item_id": "implement-task-tdl-001-1-0",
        "item": { "canonical_task_ids": ["TASK-TDL-001"] }
    });
    WorkflowV2AgentAdapter::new()
        .parse_agent_output(
            &write_request_with_input(input),
            &serde_json::to_string(&noop_result()).expect("serialize"),
        )
        .expect("plain task keeps the typed-noop path");
}

#[test]
fn empty_required_tools_array_does_not_trip_the_guard() {
    let input = serde_json::json!({
        "item": { "canonical_task_ids": ["TASK-TDL-001"], "required_tools": [] }
    });
    WorkflowV2AgentAdapter::new()
        .parse_agent_output(
            &write_request_with_input(input),
            &serde_json::to_string(&noop_result()).expect("serialize"),
        )
        .expect("an empty required_tools list is not a tool declaration");
}

fn accepted_result_json(commands: serde_json::Value) -> String {
    serde_json::json!({
        "status": "accepted",
        "summary": "implemented TASK-TDL-120 pine artifacts",
        "files_changed": [{ "path": "crates/archon-trading/src/data_lake.rs" }],
        "commands_run": commands,
        "task_coverage": [{
            "task_id": "TASK-TDL-120",
            "status": "accepted",
            "summary": "pine artifacts generated and checked",
            "evidence": [{ "kind": "implementation", "summary": "generated pine v6 artifacts" }]
        }]
    })
    .to_string()
}

#[test]
fn accepted_is_rejected_when_a_declared_required_tool_is_never_exercised() {
    // The agent ran only the offline tools and silently skipped the live-compile
    // tools the task required, asserting them "unavailable" in prose. That must
    // not stand — this is exactly the TDL-120 escape hatch.
    let input = serde_json::json!({
        "item": {
            "canonical_task_ids": ["TASK-TDL-120"],
            "required_tools": ["pine_analyze", "pine_check", "pine_compile", "pine_smart_compile"]
        }
    });
    let commands = serde_json::json!([
        { "kind": "inspect", "command": "mcp__tradingview__pine_analyze indicator", "status": "succeeded", "output_summary": "0 issues" },
        { "kind": "inspect", "command": "mcp__tradingview__pine_check strategy", "status": "succeeded", "output_summary": "compiled" }
    ]);
    let error = WorkflowV2AgentAdapter::new()
        .parse_agent_output(
            &write_request_with_input(input),
            &accepted_result_json(commands),
        )
        .expect_err("accepted must be rejected when a required tool was never invoked");
    assert!(
        matches!(
            &error,
            WorkflowV2AgentError::ImplementationAcceptedWithRequiredToolUnexercised(tool)
                if tool == "pine_compile"
        ),
        "{error}"
    );
}

#[test]
fn accepted_is_allowed_when_a_required_tool_was_attempted_even_if_it_failed() {
    // A captured FAILURE of the required tool is a genuine attempt: the honest
    // block/gap can then cite it. The guard requires an attempt, not a success.
    let input = serde_json::json!({
        "item": { "canonical_task_ids": ["TASK-TDL-120"], "required_tools": ["pine_compile"] }
    });
    let commands = serde_json::json!([
        { "kind": "other", "command": "mcp__tradingview__pine_compile on AHDM-v1-indicator.pine", "status": "failed", "output_summary": "CDP chart not reachable" }
    ]);
    WorkflowV2AgentAdapter::new()
        .parse_agent_output(
            &write_request_with_input(input),
            &accepted_result_json(commands),
        )
        .expect("a captured tool failure is a real attempt and satisfies the required-tools guard");
}

#[test]
fn accepted_with_no_required_tools_is_unaffected_by_the_exercise_guard() {
    let input = serde_json::json!({ "item": { "canonical_task_ids": ["TASK-TDL-001"] } });
    let commands = serde_json::json!([
        { "kind": "test", "command": "cargo test -p archon-trading", "status": "succeeded", "output_summary": "ok" }
    ]);
    WorkflowV2AgentAdapter::new()
        .parse_agent_output(
            &write_request_with_input(input),
            &accepted_result_json(commands),
        )
        .expect("no declared required tools means the exercise guard never trips");
}

#[test]
fn prompt_renders_project_artifact_root_from_the_typed_context_not_the_input() {
    // The verifier prompt reads project_artifact_root from request.project_artifacts
    // (the TYPED field), never from the input JSON. Stamping a policy blob into
    // the input does NOT populate it — read-only branches must be given the
    // store so this field is built. Pinning the actual consumer.
    let temp = tempfile::tempdir().expect("tempdir");
    let v2_root = temp.path().join("project/.archon/workflows/wf-x/v2");
    std::fs::create_dir_all(&v2_root).expect("v2 root");

    let mut request = write_request_with_input(serde_json::json!({
        "_workflow_project_artifact_policy": { "project_root": "/stamped/in/input/only" }
    }));
    // Input-only stamp: prompt must NOT pick it up.
    assert!(
        WorkflowV2AgentAdapter::new()
            .build_prompt(&request)
            .contains("project_artifact_root: <none>"),
        "input stamp must not satisfy the prompt"
    );

    // Typed context populated (what passing the store does): prompt resolves it.
    request.project_artifacts = crate::project_artifact_context_from_v2_root(&v2_root);
    let prompt = WorkflowV2AgentAdapter::new().build_prompt(&request);
    assert!(
        !prompt.contains("project_artifact_root: <none>"),
        "typed context must render an absolute root"
    );
}
