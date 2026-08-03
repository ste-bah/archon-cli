//! Transient-provider retry for a single workflow agent call.
//!
//! A stage agent invocation goes through one provider request; a class of
//! provider failures (stream errors, resets, 5xx, rate limits) is worth
//! retrying and the rest is not. Retrying the wrong thing is the expensive
//! mistake here — a foreground subagent that was auto-backgrounded or timed
//! out has already consumed its budget, so it is excluded explicitly.
//!
//! The client is [`crate::WorkflowLlmClient`], the port the binary supplies, so
//! nothing in this module knows which provider is behind it.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use crate::{WorkflowAgentCall, WorkflowAgentOutcome, WorkflowLlmClient};

const LIVE_AGENT_TRANSIENT_ATTEMPTS: usize = 3;

pub async fn run_agent_with_transient_retry<F, Fut>(
    llm: &Arc<dyn WorkflowLlmClient>,
    agent_request: WorkflowAgentCall,
    mut on_retry: F,
) -> crate::WorkflowResult<WorkflowAgentOutcome>
where
    F: FnMut(usize) -> Fut,
    Fut: Future<Output = crate::WorkflowResult<()>>,
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
                    on_retry(attempt).await?;
                    tokio::time::sleep(Duration::from_millis(500 * attempt as u64)).await;
                    continue;
                }
                return Err(crate::WorkflowError::StageFailed(message));
            }
        }
    }
    Err(crate::WorkflowError::StageFailed(
        last_error.unwrap_or_else(|| "transient provider retry exhausted".to_string()),
    ))
}

pub fn transient_live_agent_error(message: &str) -> bool {
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

/// The pipeline-type guard this used to carry is gone with the concrete
/// request type: every call routed through here is a workflow stage, so the
/// check was always true and only the auto-background flag ever decided.
pub fn transient_live_agent_error_for_request(request: &WorkflowAgentCall, message: &str) -> bool {
    if request.disable_auto_background && foreground_subagent_timeout_or_cancel(message) {
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
    use crate::{WorkflowAgentSpec, WorkflowAgentToolAccess};

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
}
