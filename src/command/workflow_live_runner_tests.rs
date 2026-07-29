use archon_workflow::{ProviderTier, StageKind, StageRunRequest};
use serde_json::json;

use super::workflow_live_runner::{allowed_tools, command_execution_stage};
use super::workflow_live_runner_activity::activity_detail;

fn request(input: serde_json::Value) -> StageRunRequest {
    request_with_task(input, "Implement")
}

fn request_with_task(input: serde_json::Value, task: &str) -> StageRunRequest {
    StageRunRequest {
        run_id: "wf-test".into(),
        stage_id: "stage".into(),
        stage_kind: StageKind::Implementation,
        agent: None,
        task: task.into(),
        attempt: 1,
        provider_tier: ProviderTier::Coder,
        depends_on: Vec::new(),
        input,
    }
}

#[test]
fn coordinated_implementation_object_input_gets_bash() {
    // Every implementation branch now gets Bash so the coder can build/test its
    // edits, even under write-coordination and without the task naming tests.
    let req = request(json!({"write_coordination": {"enabled": true}}));
    assert!(allowed_tools(&req).contains(&"Bash".to_string()));
}

#[test]
fn coordinated_implementation_bool_input_gets_bash() {
    let req = request(json!({"write_coordination": true}));
    assert!(allowed_tools(&req).contains(&"Bash".to_string()));
}

#[test]
fn coordinated_implementation_with_verification_gets_bash() {
    let req = request_with_task(
        json!({"write_coordination": {"enabled": true}}),
        "Implement missing work and run focused tests.",
    );
    assert!(allowed_tools(&req).contains(&"Bash".to_string()));
}

#[test]
fn activity_detail_names_stage_cwd_and_tool_mode() {
    let req = request(json!({"target_repository_root": "/tmp/project"}));
    let detail = activity_detail(&req, "stage running");

    assert!(detail.contains("stage=stage"));
    assert!(detail.contains("cwd=/tmp/project"));
    assert!(detail.contains("tool_mode=full"));
}

#[test]
fn generated_v2_read_only_verification_branch_stays_read_only() {
    let req = StageRunRequest {
        run_id: "wf-test".into(),
        stage_id: "readonly-discovery-verification-inventory".into(),
        stage_kind: StageKind::Agent,
        agent: Some("researcher".into()),
        task: "Inspect focused test commands and verification setup; do not run tests.".into(),
        attempt: 1,
        provider_tier: ProviderTier::Researcher,
        depends_on: Vec::new(),
        input: json!({
            "target_repository_root": "/tmp/project",
            "v2_call": {
                "id": "readonly-discovery-verification-inventory",
                "method": "agent",
                "role": "researcher",
                "write_mode": null,
                "target_files": []
            }
        }),
    };

    let tools = allowed_tools(&req);
    assert!(tools.contains(&"Read".to_string()));
    assert!(tools.contains(&"Grep".to_string()));
    assert!(!tools.contains(&"Bash".to_string()));
    assert!(!tools.contains(&"Write".to_string()));
    assert!(activity_detail(&req, "stage running").contains("tool_mode=read_only"));
}

#[test]
fn generated_v2_verification_wave_gets_command_execution() {
    let req = StageRunRequest {
        run_id: "wf-test".into(),
        stage_id: "verification-wave-task-verifier-2".into(),
        stage_kind: StageKind::Agent,
        agent: Some("coder".into()),
        task: "Run the declared focused verification commands.".into(),
        attempt: 1,
        provider_tier: ProviderTier::Coder,
        depends_on: Vec::new(),
        input: json!({
            "target_repository_root": "/tmp/project",
            "v2_call": {
                "id": "verification-wave-task-verifier-2",
                "method": "parallel",
                "role": "coder",
                "write_mode": null,
                "target_files": []
            }
        }),
    };

    assert!(command_execution_stage(&req));
    assert!(allowed_tools(&req).contains(&"Bash".to_string()));
}

fn mcp_project() -> tempfile::TempDir {
    let project = tempfile::tempdir().expect("temp project");
    std::fs::write(
        project.path().join(".mcp.json"),
        r#"{
          "mcpServers": {
            "tradingview": {
              "command": "node",
              "toolPolicy": {
                "trustServerHints": false,
                "toolPermissions": {
                  "data_get_ohlcv": "safe",
                  "pine_check": "safe",
                  "pine_compile": "risky",
                  "pine_smart_compile": "risky",
                  "alert_create": "dangerous"
                }
              }
            }
          }
        }"#,
    )
    .expect("write MCP config");
    project
}

fn mcp_request(project: &std::path::Path, item: serde_json::Value) -> StageRunRequest {
    request(json!({
        "project_artifact_root": project,
        "item": item
    }))
}

#[test]
fn d37_declared_provider_and_pine_tools_are_exposed() {
    let project = mcp_project();
    let provider = mcp_request(
        project.path(),
        json!({
            "canonical_task_ids": ["TASK-DEMO-017"],
            "required_tools": ["data_get_ohlcv"]
        }),
    );
    let pine = mcp_request(
        project.path(),
        json!({
            "canonical_task_ids": ["TASK-DEMO-023"],
            "required_tools": ["pine_compile", "pine_smart_compile"]
        }),
    );

    let provider_tools = allowed_tools(&provider);
    let pine_tools = allowed_tools(&pine);
    assert!(provider_tools.contains(&"mcp__tradingview__data_get_ohlcv".to_string()));
    assert!(!provider_tools.contains(&"mcp__tradingview__pine_compile".to_string()));
    assert!(pine_tools.contains(&"mcp__tradingview__pine_compile".to_string()));
    assert!(pine_tools.contains(&"mcp__tradingview__pine_smart_compile".to_string()));
    assert!(!pine_tools.contains(&"mcp__tradingview__data_get_ohlcv".to_string()));
}

#[test]
fn d37_declared_tools_are_honored_but_dangerous_policy_is_not() {
    let project = mcp_project();
    let req = mcp_request(
        project.path(),
        json!({
            "canonical_task_ids": ["TASK-DEMO-001"],
            "required_tools": ["pine_check", "alert_create"]
        }),
    );

    let tools = allowed_tools(&req);
    assert!(tools.contains(&"mcp__tradingview__pine_check".to_string()));
    assert!(!tools.contains(&"mcp__tradingview__alert_create".to_string()));
}
