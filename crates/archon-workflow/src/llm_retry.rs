//! Bounded transient-failure retry around [`crate::llm_client_port`].
//!
//! A provider call can fail for two unrelated reasons: the request was wrong,
//! or the transport blinked. Only the second is worth repeating, and repeating
//! the first burns a run's budget on an answer that will not change. The
//! classifier below is the single place that decides which one happened, so
//! every surface that dispatches through the LLM port retries on the same rule.
//!
//! This sits beside the port rather than in the host for the same reason the
//! port itself does: the retry budget and the transient-error vocabulary are
//! facts about how this crate uses a provider, not about which provider the
//! host happened to install.
//!
//! `on_retry` is a required notification, not a log line. It returns a
//! `WorkflowResult` and its failure ABORTS the retry: a run that cannot tell
//! the operator it is retrying must not silently retry.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use crate::error::{WorkflowError, WorkflowResult};
use crate::llm_client_port::{WorkflowAgentCall, WorkflowAgentOutcome, WorkflowLlmClient};

const LIVE_AGENT_TRANSIENT_ATTEMPTS: usize = 3;

pub async fn run_agent_with_transient_retry<F, Fut>(
    llm: &Arc<dyn WorkflowLlmClient>,
    agent_request: WorkflowAgentCall,
    mut on_retry: F,
) -> WorkflowResult<WorkflowAgentOutcome>
where
    F: FnMut(usize) -> Fut,
    Fut: Future<Output = WorkflowResult<()>>,
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
                return Err(WorkflowError::StageFailed(message));
            }
        }
    }
    Err(WorkflowError::StageFailed(last_error.unwrap_or_else(
        || "transient provider retry exhausted".to_string(),
    )))
}

pub async fn send_message_with_transient_retry<F, Fut>(
    llm: &Arc<dyn WorkflowLlmClient>,
    messages: Vec<Value>,
    system: Vec<Value>,
    tools: Vec<Value>,
    model: &str,
    mut on_retry: F,
) -> WorkflowResult<WorkflowAgentOutcome>
where
    F: FnMut(usize) -> Fut,
    Fut: Future<Output = WorkflowResult<()>>,
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
                    last_error = Some(error);
                    on_retry(attempt).await?;
                    tokio::time::sleep(Duration::from_millis(500 * attempt as u64)).await;
                    continue;
                }
                return Err(error);
            }
        }
    }
    // Unreachable: the final attempt above always returns. Carrying the last
    // provider error rather than a fresh string keeps the reported message
    // identical to what the caller would have seen without the retry wrapper.
    Err(last_error.unwrap_or_else(|| {
        WorkflowError::StageFailed("transient provider retry exhausted".to_string())
    }))
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
#[path = "llm_retry_tests.rs"]
mod tests;
