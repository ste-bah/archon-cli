use super::*;
use crate::llm_client_port::{WorkflowAgentSpec, WorkflowAgentToolAccess};

#[test]
fn workflow_foreground_subagent_timeout_is_not_transient_retry() {
    let request = request(true);

    assert!(!transient_live_agent_error_for_request(
        &request,
        "subagent timed out after 7200s"
    ));
    assert!(transient_live_agent_error_for_request(
        &request,
        "provider request timed out"
    ));
}

#[test]
fn auto_background_timeout_keeps_provider_retry_behavior() {
    let request = request(false);

    assert!(transient_live_agent_error_for_request(
        &request,
        "subagent timed out after 30s"
    ));
}

fn request(disable_auto_background: bool) -> WorkflowAgentCall {
    WorkflowAgentCall {
        session_id: "run".to_string(),
        task: "branch".to_string(),
        cwd: None,
        ordinal: 1,
        attempt: 1,
        agent: WorkflowAgentSpec {
            key: "coder".to_string(),
            display_name: "Coder".to_string(),
            model: "sonnet".to_string(),
            phase: 1,
            critical: true,
            parallelizable: true,
            quality_threshold: 0.8,
            tool_access: WorkflowAgentToolAccess::Full,
        },
        messages: Vec::new(),
        system: Vec::new(),
        tools: Vec::new(),
        allowed_tools: Vec::new(),
        timeout_secs: Some(7200),
        disable_auto_background,
        provider_env: None,
    }
}
