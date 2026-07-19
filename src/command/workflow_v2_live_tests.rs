use archon_workflow::{
    ProviderTier, WorkflowV2AgentAdapter, WorkflowV2AgentRequest, WorkflowV2HostCall,
    WorkflowV2HostMethod, WorkflowV2WriteMode,
};

use super::workflow_live_v2::provider_tier_for_v2_request;

#[test]
fn cli_v2_prompt_contract_uses_typed_adapter_contract() {
    let prompt = WorkflowV2AgentAdapter::new().build_prompt(&WorkflowV2AgentRequest {
        call: WorkflowV2HostCall {
            id: "implement".into(),
            method: WorkflowV2HostMethod::Agent,
            write_mode: Some(WorkflowV2WriteMode::Serial),
            options: Default::default(),
        },
        role: "coder".into(),
        task: "Implement the requested slice".into(),
        constraints: vec!["no plan-only result".into()],
        input: serde_json::json!({ "task_id": "T001" }),
        repository_root: Some("/repo".into()),
        project_artifacts: Default::default(),
        target_files: vec!["src/lib.rs".into()],
        target_ownership_scopes: Vec::new(),
    });

    assert!(prompt.contains("Required JSON Result Envelope"));
    assert!(prompt.contains("files_changed must list each changed path"));
    assert!(prompt.contains("\"task_coverage\""));
    assert!(!prompt.to_ascii_lowercase().contains("openai"));
    assert!(!prompt.to_ascii_lowercase().contains("claude"));
}

#[test]
fn cli_v2_live_tier_selection_uses_host_call_role_metadata() {
    assert_eq!(
        provider_tier_for_v2_request(&request("critic")),
        ProviderTier::Critic
    );
    assert_eq!(
        provider_tier_for_v2_request(&request("reducer")),
        ProviderTier::Reducer
    );
    assert_eq!(
        provider_tier_for_v2_request(&request("planner")),
        ProviderTier::Planner
    );
    assert_eq!(
        provider_tier_for_v2_request(&request("researcher")),
        ProviderTier::Researcher
    );
    assert_eq!(
        provider_tier_for_v2_request(&request("coder")),
        ProviderTier::Coder
    );
}

fn request(role: &str) -> WorkflowV2AgentRequest {
    WorkflowV2AgentRequest {
        call: WorkflowV2HostCall {
            id: format!("{role}-call"),
            method: WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: Default::default(),
        },
        role: role.to_string(),
        task: "Execute typed host call".into(),
        constraints: Vec::new(),
        input: serde_json::json!({}),
        repository_root: None,
        project_artifacts: Default::default(),
        target_files: Vec::new(),
        target_ownership_scopes: Vec::new(),
    }
}
