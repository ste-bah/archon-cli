use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow};
use archon_pipeline::runner::{AgentExecutionRequest, LlmClient, LlmResponse, PipelineType};
use serde_json::Value;

const LIVE_AGENT_TRANSIENT_ATTEMPTS: usize = 3;

pub(crate) async fn run_agent_with_transient_retry<F>(
    llm: &Arc<dyn LlmClient>,
    agent_request: AgentExecutionRequest,
    mut on_retry: F,
) -> archon_workflow::WorkflowResult<LlmResponse>
where
    F: FnMut(usize),
{
    let mut last_error = None;
    for attempt in 1..=LIVE_AGENT_TRANSIENT_ATTEMPTS {
        match llm.run_agent(agent_request.clone()).await {
            Ok(response) => return Ok(response),
            Err(error) => {
                let message = error.to_string();
                if attempt < LIVE_AGENT_TRANSIENT_ATTEMPTS
                    && transient_live_agent_error_for_request(&agent_request, &message)
                {
                    last_error = Some(message);
                    on_retry(attempt);
                    tokio::time::sleep(Duration::from_millis(500 * attempt as u64)).await;
                    continue;
                }
                return Err(archon_workflow::WorkflowError::StageFailed(message));
            }
        }
    }
    Err(archon_workflow::WorkflowError::StageFailed(
        last_error.unwrap_or_else(|| "transient provider retry exhausted".to_string()),
    ))
}

pub(crate) async fn send_message_with_transient_retry<F>(
    llm: &Arc<dyn LlmClient>,
    messages: Vec<Value>,
    system: Vec<Value>,
    tools: Vec<Value>,
    model: &str,
    mut on_retry: F,
) -> Result<LlmResponse>
where
    F: FnMut(usize),
{
    let mut last_error = None;
    for attempt in 1..=LIVE_AGENT_TRANSIENT_ATTEMPTS {
        match llm
            .send_message(messages.clone(), system.clone(), tools.clone(), model)
            .await
        {
            Ok(response) => return Ok(response),
            Err(error) => {
                let message = error.to_string();
                if attempt < LIVE_AGENT_TRANSIENT_ATTEMPTS && transient_live_agent_error(&message) {
                    last_error = Some(message);
                    on_retry(attempt);
                    tokio::time::sleep(Duration::from_millis(500 * attempt as u64)).await;
                    continue;
                }
                return Err(error);
            }
        }
    }
    Err(anyhow!(
        "{}",
        last_error.unwrap_or_else(|| "transient provider retry exhausted".to_string())
    ))
}

pub(crate) fn transient_live_agent_error(message: &str) -> bool {
    let text = message.to_ascii_lowercase();
    [
        "llm stream error",
        "server_error",
        "error decoding response body",
        "error sending request",
        "request failed",
        "connection reset",
        "connection closed",
        "connection refused",
        "broken pipe",
        "timed out",
        "timeout",
        "temporar",
        "rate limit",
        "429",
        "500",
        "502",
        "503",
        "504",
    ]
    .iter()
    .any(|needle| text.contains(needle))
}

pub(crate) fn transient_live_agent_error_for_request(
    request: &AgentExecutionRequest,
    message: &str,
) -> bool {
    if request.pipeline_type == PipelineType::Workflow
        && request.disable_auto_background
        && foreground_subagent_timeout_or_cancel(message)
    {
        return false;
    }
    transient_live_agent_error(message)
}

fn foreground_subagent_timeout_or_cancel(message: &str) -> bool {
    let text = message.to_ascii_lowercase();
    text.contains("subagent timed out")
        || text.contains("subagent cancelled")
        || text.contains("subagent auto-backgrounded")
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_pipeline::runner::{AgentInfo, ToolAccessLevel};

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

    fn request(disable_auto_background: bool) -> AgentExecutionRequest {
        AgentExecutionRequest {
            session_id: "run".to_string(),
            pipeline_type: PipelineType::Workflow,
            task: "branch".to_string(),
            cwd: None,
            ordinal: 1,
            attempt: 1,
            agent: AgentInfo {
                key: "coder".to_string(),
                display_name: "Coder".to_string(),
                model: "sonnet".to_string(),
                phase: 1,
                critical: true,
                parallelizable: true,
                quality_threshold: 0.8,
                tool_access_level: ToolAccessLevel::Full,
            },
            messages: Vec::new(),
            system: Vec::new(),
            tools: Vec::new(),
            allowed_tools: Vec::new(),
            timeout_secs: Some(7200),
            disable_auto_background,
        }
    }
}
