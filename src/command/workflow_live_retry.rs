//! What is left of live-workflow retry in the binary.
//!
//! The agent-call retry moved to `archon_workflow::stage_retry` with the
//! `WorkflowLlmClient` port it drives. This raw `send_message` variant stays
//! because it is anyhow-typed all the way through its caller's retry closure,
//! and archon-workflow deliberately carries no anyhow dependency; the transient
//! classifier is shared from the crate so the two paths cannot drift.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow};
use archon_workflow::{WorkflowAgentOutcome, WorkflowLlmClient};
use serde_json::Value;

use archon_workflow::stage_retry::transient_live_agent_error;

const LIVE_AGENT_TRANSIENT_ATTEMPTS: usize = 3;

pub(crate) async fn send_message_with_transient_retry<F, Fut>(
    llm: &Arc<dyn WorkflowLlmClient>,
    messages: Vec<Value>,
    system: Vec<Value>,
    tools: Vec<Value>,
    model: &str,
    mut on_retry: F,
) -> Result<WorkflowAgentOutcome>
where
    F: FnMut(usize) -> Fut,
    Fut: Future<Output = Result<()>>,
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
                    on_retry(attempt).await?;
                    tokio::time::sleep(Duration::from_millis(500 * attempt as u64)).await;
                    continue;
                }
                return Err(error.into());
            }
        }
    }
    Err(anyhow!(
        "{}",
        last_error.unwrap_or_else(|| "transient provider retry exhausted".to_string())
    ))
}
