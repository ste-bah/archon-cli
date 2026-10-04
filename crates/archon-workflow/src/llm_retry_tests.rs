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

/// An inactivity cut says "timeout" too, and is the host's decision exactly
/// like the wall clock: never a provider blink to re-ask.
#[test]
fn workflow_inactivity_cut_is_not_transient_retry() {
    let text = format!(
        "agent transport failed: {} no model output, tool call or tool result for 1800s",
        crate::error::INACTIVITY_TIMEOUT_MARKER
    );
    assert!(
        transient_live_agent_error(&text),
        "the bare classifier matches"
    );
    assert!(!transient_live_agent_error_for_request(
        &request(true),
        &text
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
        read_roots: Vec::new(),
        write_roots: Vec::new(),
        provider_env: None,
    }
}

/// A port client that cannot restore a completed invocation refuses to
/// continue one instead of answering with a fresh call (#241).
#[tokio::test]
async fn a_port_client_that_cannot_restore_refuses_to_continue() {
    struct Stateless;
    #[async_trait::async_trait]
    impl crate::llm_client_port::WorkflowLlmClient for Stateless {
        async fn send_message(
            &self,
            _: Vec<serde_json::Value>,
            _: Vec<serde_json::Value>,
            _: Vec<serde_json::Value>,
            _: &str,
        ) -> crate::error::WorkflowResult<crate::llm_client_port::WorkflowAgentOutcome> {
            panic!("a continuation was answered with a fresh call")
        }
    }
    use crate::llm_client_port::WorkflowLlmClient;
    let error = Stateless
        .continue_agent(request(true))
        .await
        .expect_err("a stateless client continued");
    assert!(error.to_string().contains("start a new agent"), "{error}");
}
