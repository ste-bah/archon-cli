// Issue-47: the host's built-in agent tools are not capabilities the
// required-tool proof polices, and the proof matches by token, not substring.
//
// Sibling of `agent_adapter_required_tools_tests.rs` (the Issue-39/40 cases),
// split out for the 500-line ceiling.

use super::*;
use crate::{
    WorkflowV2CommandKind, WorkflowV2CommandStatus, WorkflowV2Evidence, WorkflowV2EvidenceKind,
    WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions, WorkflowV2TaskCoverage,
    WorkflowV2TaskCoverageStatus, WorkflowV2WriteMode,
};

fn write_request(required_tools: serde_json::Value) -> WorkflowV2AgentRequest {
    WorkflowV2AgentRequest {
        call: WorkflowV2HostCall {
            id: "implement-task-1-0".to_string(),
            method: WorkflowV2HostMethod::Implementation,
            write_mode: Some(WorkflowV2WriteMode::Coordinated),
            options: WorkflowV2HostOptions::default(),
        },
        role: "coder".to_string(),
        task: "Implement TASK-001".to_string(),
        constraints: Vec::new(),
        input: serde_json::json!({
            "item": { "canonical_task_ids": ["TASK-001"], "required_tools": required_tools }
        }),
        repository_root: Some("/repo".to_string()),
        project_artifacts: Default::default(),
        target_files: vec!["src/lib.rs".to_string()],
        target_ownership_scopes: Vec::new(),
    }
}

fn accepted_result_json(commands: serde_json::Value) -> String {
    serde_json::json!({
        "status": "accepted",
        "summary": "implemented TASK-001",
        "files_changed": [{ "path": "src/lib.rs" }],
        "commands_run": commands,
        "task_coverage": [{
            "task_id": "TASK-001",
            "status": "accepted",
            "summary": "implemented and checked",
            "evidence": [{ "kind": "implementation", "summary": "wrote src/lib.rs" }]
        }]
    })
    .to_string()
}

fn cargo_only_commands() -> serde_json::Value {
    serde_json::json!([
        { "kind": "build", "command": "cargo build -p demo", "status": "succeeded", "output_summary": "ok" },
        { "kind": "test", "command": "cargo test -p demo", "status": "succeeded", "output_summary": "12 passed" }
    ])
}

fn captured(command: &str) -> WorkflowV2CommandRecord {
    WorkflowV2CommandRecord {
        kind: WorkflowV2CommandKind::Other,
        command: command.to_string(),
        status: WorkflowV2CommandStatus::Succeeded,
        exit_code: Some(0),
        output_summary: "ok".to_string(),
        pre_existing: false,
    }
}

fn unexercised(required_tools: serde_json::Value, commands: &[&str]) -> Vec<String> {
    let mut result = WorkflowV2Result::accepted("done");
    result.commands_run = commands.iter().map(|command| captured(command)).collect();
    super::unexercised_required_tools(&write_request(required_tools).input, &result)
}

/// (a) Live, both branches of a wave finished and were refused for
/// "edit, glob, read, write": built-in tool uses are host observations, not
/// commands, so they can never appear in `commands_run` by construction.
#[test]
fn builtin_agent_tools_are_not_policed_by_the_exercise_proof() {
    WorkflowV2AgentAdapter::new()
        .parse_agent_output(
            &write_request(serde_json::json!(["Read", "Write", "Edit", "Glob", "Grep"])),
            &accepted_result_json(cargo_only_commands()),
        )
        .expect("built-in file and search tools are not capabilities the proof demands");
}

/// (b) The exemption is exact: an MCP tool declared beside built-ins is still
/// policed, and the rejection names only it, by the name the task declared.
#[test]
fn an_mcp_tool_beside_builtins_is_still_policed_and_named_alone() {
    let error = WorkflowV2AgentAdapter::new()
        .parse_agent_output(
            &write_request(serde_json::json!(["Read", "mcp__x__y"])),
            &accepted_result_json(cargo_only_commands()),
        )
        .expect_err("an MCP tool must still be exercised");
    assert!(
        matches!(
            &error,
            WorkflowV2AgentError::ImplementationAcceptedWithRequiredToolUnexercised(tools)
                if tools == &["mcp__x__y".to_string()]
        ),
        "{error}"
    );
}

/// (c) Substring matching proved a required `read` with `readiness`. A token
/// must equal the name.
#[test]
fn a_required_tool_is_not_proven_by_a_command_that_merely_contains_its_name() {
    assert_eq!(
        unexercised(
            serde_json::json!(["sync"]),
            &["cargo test readiness", "rsync -a src/ dst/"]
        ),
        vec!["sync".to_string()]
    );
    assert!(unexercised(serde_json::json!(["sync"]), &["sync --all"]).is_empty());
    assert!(unexercised(serde_json::json!(["sync"]), &["archon call sync"]).is_empty());
    // `-` stays inside a token: `read-only` is not `read`, `ledger-sync` is not `sync`.
    assert_eq!(
        unexercised(
            serde_json::json!(["mcp__srv__sync"]),
            &["ledger-sync --dry-run"]
        ),
        vec!["mcp__srv__sync".to_string()]
    );
}

/// (c) An MCP server may export a tool that shares a built-in's name. Declared
/// qualified, it is a capability and stays policed — and `readiness` no
/// longer proves it; only a `read` token does.
#[test]
fn a_qualified_mcp_tool_named_like_a_builtin_is_still_policed() {
    assert_eq!(
        unexercised(
            serde_json::json!(["mcp__srv__read"]),
            &["cargo test readiness"]
        ),
        vec!["mcp__srv__read".to_string()]
    );
    assert!(
        unexercised(
            serde_json::json!(["mcp__srv__read"]),
            &["mcp__srv__read {}"]
        )
        .is_empty()
    );
    assert!(
        unexercised(
            serde_json::json!(["mcp__srv__read"]),
            &["archon call srv read"]
        )
        .is_empty()
    );
}

/// (d) The wire form with a JSON argument, as agents record MCP calls.
#[test]
fn an_mcp_tool_is_proven_by_its_wire_form_invocation_with_captured_output() {
    assert!(
        unexercised(
            serde_json::json!(["mcp__x__y"]),
            &[r#"mcp__x__y {"symbol":"ABC","limit":5}"#]
        )
        .is_empty()
    );
    // And by the `mcp_action:` spelling, which reduces the same way.
    assert!(unexercised(serde_json::json!(["mcp__x__y"]), &["mcp_action:y ABC"]).is_empty());
}

fn noop_result_json() -> String {
    let mut result = WorkflowV2Result {
        status: WorkflowV2Status::Noop,
        summary: "already complete".to_string(),
        ..WorkflowV2Result::default()
    };
    result.task_coverage.push(WorkflowV2TaskCoverage {
        task_id: "TASK-001".to_string(),
        status: WorkflowV2TaskCoverageStatus::Noop,
        summary: "existing implementation inspected".to_string(),
        evidence: vec![WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Inspection,
            "read src/lib.rs; the declared behaviour is already present",
        )],
    });
    serde_json::to_string(&result).expect("serialize")
}

/// (e) A task whose only required tools are built-ins may no-op with typed
/// coverage evidence, exactly like a task that declares none.
#[test]
fn noop_is_allowed_when_only_builtin_tools_are_required() {
    WorkflowV2AgentAdapter::new()
        .parse_agent_output(
            &write_request(serde_json::json!(["Read", "Grep", "Glob", "Edit"])),
            &noop_result_json(),
        )
        .expect("built-in tools are how the cited inspection was done");
}

/// (e) An MCP tool must still be exercised this run; a no-op is refused as it
/// always was.
#[test]
fn noop_is_still_refused_when_an_mcp_tool_is_required() {
    let error = WorkflowV2AgentAdapter::new()
        .parse_agent_output(
            &write_request(serde_json::json!(["Read", "mcp__x__y"])),
            &noop_result_json(),
        )
        .expect_err("an MCP tool cannot be satisfied by a no-op");
    assert!(
        matches!(
            error,
            WorkflowV2AgentError::ImplementationNoopWithDeclaredRequiredTools
        ),
        "{error}"
    );
}

/// The exemption is case-insensitive, as tool names are declared in either
/// case, and covers the host's own spellings alongside the common aliases.
#[test]
fn builtin_names_match_case_insensitively() {
    assert!(super::is_builtin_agent_tool("read"));
    assert!(super::is_builtin_agent_tool("WEBFETCH"));
    assert!(super::is_builtin_agent_tool("ApplyPatch"));
    assert!(!super::is_builtin_agent_tool("memory_store"));
    assert!(!super::is_builtin_agent_tool("ReadMcpResource"));
}
