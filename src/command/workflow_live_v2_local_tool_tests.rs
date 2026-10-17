use super::*;
use archon_workflow::{WorkflowV2HostCall, WorkflowV2HostOptions};

#[test]
fn declared_checkpoint_tool_delegates_to_local_host_api() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let execution = tool_execution("checkpoint-tool", "checkpoint");

    let result =
        execute_declared_local_tool(execution, &store, None).expect("checkpoint tool result");

    assert_eq!(result.status, WorkflowV2Status::Accepted);
    assert!(result.summary.contains("checkpoint-tool"));
}

#[test]
fn undeclared_tool_fails_closed_before_execution() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let execution = tool_execution("mystery-tool", "shell");

    let err = execute_declared_local_tool(execution, &store, None)
        .expect_err("unknown tools must fail closed");

    assert!(err.to_string().contains("unknown local tool"));
}

fn tool_execution(id: &str, tool: &str) -> WorkflowV2CallExecution {
    let mut options = WorkflowV2HostOptions::default();
    options
        .extra
        .insert("tool".to_string(), serde_json::json!(tool));
    WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            id: id.to_string(),
            method: WorkflowV2HostMethod::Tool,
            write_mode: None,
            options,
        },
        input: serde_json::json!({
            "options": {
                "tool": tool,
                "inputs": {
                    "payload": id
                }
            },
            "source_data": {
                "payload": id
            }
        }),
        depends_on: Vec::new(),
    }
}
