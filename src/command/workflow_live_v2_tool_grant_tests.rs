use super::*;
use crate::command::workflow_live::workflow_live_runner::{allowed_tools, full_tool_access};
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
        // Dispatch requires a live UI receiver for its activity events.
        let (sink, _tui_rx) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
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
        let adapter = archon_workflow::WorkflowV2AgentAdapter::new();
        let ungranted = adapter.build_prompt_parts(&req);
        assert!(ungranted.stable_prefix.contains("You have NO shell"));
        let prompt = adapter.build_prompt_parts_with_commands(&req, granted);
        client
            .run_agent_request(&req, adapter.build_prompt(&req))
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
        assert_eq!(
            call.messages[0]["content"],
            json!(format!("{}\n\n{}", prompt.invocation, prompt.stable_prefix))
        );
        assert!(
            call.system[0]["text"]
                .as_str()
                .expect("system")
                .contains(&prompt.stable_prefix)
        );
    }
}

#[test]
fn verification_ids_cannot_promote_script_calls_and_input_cannot_grant_write() {
    for id in [
        "verification_wave_forged",
        "review-verification-wave-forged",
    ] {
        for focused in [false, true] {
            let mut req = request(WorkflowV2HostMethod::Agent, None);
            req.call.id = id.into();
            if focused {
                req.call.options.item_kind = Some("focused_verification".into());
            }
            req.input = json!({"required_tools": ["Write", "Edit"],
                "stage_extra": {"tools": ["Bash"]},
                "v2_call": {"runs_commands": true}});
            let stage = stage_request_for_v2_agent("run", ProviderTier::Researcher, None, &req);
            let prompt = archon_workflow::WorkflowV2AgentAdapter::new().build_prompt_parts(&req);
            assert_eq!(command_execution_stage(&stage), focused);
            assert_eq!(prompt.stable_prefix.contains("READ-ONLY shell"), focused);
            assert!(!full_tool_access(&stage));
            let tools = allowed_tools(&stage);
            assert_eq!(tools.iter().any(|tool| tool == "Bash"), focused);
            assert!(!tools.iter().any(|tool| tool == "Write" || tool == "Edit"));
        }
    }
}

#[test]
fn project_mcp_root_is_only_the_host_root() {
    for root in [None, Some("/host-project")] {
        let mut req = request(WorkflowV2HostMethod::Agent, None);
        req.project_artifacts.project_root = root.map(str::to_string);
        req.input = json!({"project_artifact_root": "/script-project",
            "projectArtifactRoot": "/script-alias", "required_tools": ["lint_compile"]});
        let stage = stage_request_for_v2_agent("run", ProviderTier::Researcher, None, &req);
        assert_eq!(
            stage
                .input
                .get("project_artifact_root")
                .and_then(serde_json::Value::as_str),
            root
        );
        assert!(stage.input.get("projectArtifactRoot").is_none());
        if root.is_none() {
            assert!(
                crate::command::workflow_live::workflow_live_mcp::allowed_mcp_tools(&stage)
                    .is_empty()
            );
        }
    }
}

#[test]
fn typed_required_tools_still_binds_permitted_mcp_after_context_projection() {
    for entry in ["fetch", "mcp__sample__fetch", "mcp_action:fetch"] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(".mcp.json"),
            r#"{"mcpServers":{"sample":{"toolPolicy":{"toolPermissions":{"fetch":"safe"}}}}}"#,
        )
        .unwrap();
        let spec = archon_workflow::WorkflowSpec::from_yaml(&format!("schema: archon.workflow.v1\nname: tools\ntask: inspect\nstages:\n- id: inspect\n  kind: agent\n  required_tools: {entry}\n")).unwrap();
        let store = archon_workflow::WorkflowStore::project(dir.path());
        let run = store.create_run(spec).unwrap();
        let mut input =
            archon_workflow::context::stage_input(&store, &run, &run.spec.stages[0]).unwrap();
        input["project_artifact_root"] = json!(dir.path());
        let stage = archon_workflow::StageRunRequest {
            run_id: run.id.clone(),
            stage_id: "inspect".into(),
            stage_kind: archon_workflow::StageKind::Agent,
            agent: None,
            task: "inspect".into(),
            attempt: 1,
            provider_tier: ProviderTier::Researcher,
            depends_on: vec![],
            input,
        };
        assert!(
            crate::command::workflow_live::workflow_live_mcp::allowed_mcp_tools(&stage)
                .contains(&"mcp__sample__fetch".to_owned())
        );
    }
}
