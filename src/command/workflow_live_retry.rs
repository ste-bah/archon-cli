use std::sync::Arc;
use std::time::Duration;

use archon_pipeline::runner::{AgentExecutionRequest, LlmClient, LlmResponse};

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
                if attempt < LIVE_AGENT_TRANSIENT_ATTEMPTS && transient_live_agent_error(&message) {
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

pub(crate) fn transient_live_agent_error(message: &str) -> bool {
    let text = message.to_ascii_lowercase();
    [
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
