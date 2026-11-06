use super::*;
use crate::command::workflow_live_runner::{allowed_tools, full_tool_access};
use archon_workflow::stage_command_policy::{command_execution_stage, read_only_shell_grant};
use serde_json::json;

fn client() -> LiveV2AgentClient {
    let (sink, _) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    LiveV2AgentClient::new(
        Arc::new(RecordingClient::default()),
        sink,
        Vec::new(),
        "grant-test".into(),
        Some("/repo".into()),
        None,
    )
}

#[test]
fn script_input_cannot_grant_a_read_only_call_shell_or_write_tools() {
    for extra in [
        json!({"tools": ["Bash", "Write", "Edit"]}),
        json!({"allowed_tools": ["Bash"], "read_only_shell": true}),
        json!({"required_tools": ["Bash"], "commands": true}),
    ] {
        let mut req = request(WorkflowV2HostMethod::Agent, None);
        req.input = json!({
            "stage_extra": extra, "read_only_shell": true,
            "v2_call": {"method": "implementation"},
            "write_coordination": {"enabled": true},
            "required_tools": ["Write", "Edit"], "objective": "inspect"
        });
        let original = req.clone();
        let stage = stage_request_for_v2_agent("run", ProviderTier::Researcher, None, &req);
        assert!(!command_execution_stage(&stage));
        assert!(!full_tool_access(&stage));
        let tools = allowed_tools(&stage);
        for tool in ["Bash", "Write", "Edit"] {
            assert!(!tools.iter().any(|name| name == tool), "{tools:?}");
        }
        for key in ["stage_extra", "read_only_shell", "write_coordination"] {
            assert!(stage.input.get(key).is_none(), "{key}");
        }
        assert_eq!(stage.input["objective"], json!("inspect"));
        assert_eq!(
            req, original,
            "stage projection preserves typed prompt input"
        );
    }
}

#[test]
fn host_artifact_grant_is_added_after_script_grants_are_removed() {
    let mut req = request(
        WorkflowV2HostMethod::Implementation,
        Some(WorkflowV2WriteMode::Serial),
    );
    req.project_artifacts.project_root = Some("/project".into());
    req.input = json!({"stage_extra": "cannot obstruct the host"});
    let stage = stage_request_for_v2_agent("run", ProviderTier::Coder, None, &req);
    assert_eq!(
        stage.input["stage_extra"]["required_tools"],
        json!(["Bash"])
    );
    assert!(allowed_tools(&stage).iter().any(|tool| tool == "Bash"));
}

#[test]
fn review_shell_requires_host_client_grant_after_projection() {
    let mut req = request(WorkflowV2HostMethod::Agent, None);
    req.input = json!({"stage_extra": {
        "read_only_shell": true, "allowed_tools": ["Bash", "Write", "Edit"]
    }});
    for granted in [false, true] {
        let mut stage = stage_request_for_v2_agent("run", ProviderTier::Researcher, None, &req);
        client()
            .with_review_shell(granted)
            .apply_review_shell_grant(&mut stage);
        assert_eq!(command_execution_stage(&stage), granted);
        assert_eq!(read_only_shell_grant(&stage.input), granted);
        assert!(!full_tool_access(&stage));
        let tools = allowed_tools(&stage);
        assert_eq!(tools.iter().any(|tool| tool == "Bash"), granted);
        assert!(!tools.iter().any(|tool| tool == "Write" || tool == "Edit"));
    }
}

#[tokio::test]
async fn dispatch_uses_host_review_state_instead_of_script_grants() {
    for granted in [false, true] {
        let recorder = Arc::new(RecordingClient::default());
        let (sink, _) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
        let client = LiveV2AgentClient::new(
            recorder.clone(),
            sink,
            Vec::new(),
            "grant-test".into(),
            None,
            None,
        )
        .with_review_shell(granted);
        let mut req = request(WorkflowV2HostMethod::Agent, None);
        req.input = json!({"stage_extra": {"tools": ["Bash", "Write", "Edit"]}});
        let prompt = archon_workflow::WorkflowV2AgentAdapter::new().build_prompt_parts(&req);
        client
            .run_agent_request(&req, prompt.invocation.clone())
            .await
            .expect("dispatch");
        let call = recorder
            .last_request
            .lock()
            .expect("recording lock")
            .clone()
            .expect("call");
        assert_eq!(
            call.allowed_tools.iter().any(|tool| tool == "Bash"),
            granted
        );
        assert!(
            !call
                .allowed_tools
                .iter()
                .any(|tool| tool == "Write" || tool == "Edit")
        );
        assert_eq!(
            call.agent.tool_access,
            archon_workflow::WorkflowAgentToolAccess::ReadOnly
        );
        assert_eq!(call.messages[0]["content"], json!(prompt.invocation));
        assert!(
            call.system[0]["text"]
                .as_str()
                .expect("system")
                .contains(&prompt.stable_prefix)
        );
    }
}
