//! Transient-failure retry around [`crate::llm_client_port`].
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

const INITIAL_AGENT_RETRY_DELAY: Duration = Duration::from_millis(500);
const MAX_AGENT_RETRY_DELAY: Duration = Duration::from_secs(8);
const LIVE_AGENT_TRANSIENT_ATTEMPTS: usize = 3;

pub async fn run_agent_with_transient_retry<F, Fut>(
    llm: &Arc<dyn WorkflowLlmClient>,
    agent_request: WorkflowAgentCall,
    on_retry: F,
) -> WorkflowResult<WorkflowAgentOutcome>
where
    F: FnMut(usize) -> Fut,
    Fut: Future<Output = WorkflowResult<()>>,
{
    agent_with_transient_retry(llm, agent_request, false, on_retry).await
}

pub async fn continue_agent_with_transient_retry<F, Fut>(
    llm: &Arc<dyn WorkflowLlmClient>,
    agent_request: WorkflowAgentCall,
    on_retry: F,
) -> WorkflowResult<WorkflowAgentOutcome>
where
    F: FnMut(usize) -> Fut,
    Fut: Future<Output = WorkflowResult<()>>,
{
    agent_with_transient_retry(llm, agent_request, true, on_retry).await
}

async fn agent_with_transient_retry<F, Fut>(
    llm: &Arc<dyn WorkflowLlmClient>,
    agent_request: WorkflowAgentCall,
    continuing: bool,
    on_retry: F,
) -> WorkflowResult<WorkflowAgentOutcome>
where
    F: FnMut(usize) -> Fut,
    Fut: Future<Output = WorkflowResult<()>>,
{
    let work = archon_tools::provider_recovery::scope(run_agent_retry_loop(
        llm,
        agent_request,
        continuing,
        on_retry,
    ));
    if archon_tools::subagent_dispatch_clock::current_call().is_some() {
        work.await
    } else {
        // Session retries still belong to one logical dispatch. Its existing
        // host no-progress bound must retain time spent in earlier sessions.
        archon_tools::subagent_dispatch_clock::scope_call(
            archon_tools::subagent_dispatch_clock::DispatchClock::new(),
            work,
        )
        .await
    }
}

async fn run_agent_retry_loop<F, Fut>(
    llm: &Arc<dyn WorkflowLlmClient>,
    agent_request: WorkflowAgentCall,
    continuing: bool,
    mut on_retry: F,
) -> WorkflowResult<WorkflowAgentOutcome>
where
    F: FnMut(usize) -> Fut,
    Fut: Future<Output = WorkflowResult<()>>,
{
    let mut attempt = 1usize;
    loop {
        let clock = archon_tools::subagent_dispatch_clock::current_call();
        let spent = clock
            .as_ref()
            .map(|clock| clock.elapsed())
            .unwrap_or_default();
        let response = if continuing {
            llm.continue_agent(agent_request.clone()).await
        } else {
            llm.run_agent(agent_request.clone()).await
        };
        match response {
            Ok(response) => return Ok(response),
            // Issue 324: a host fault is neither retried nor re-typed.
            Err(error) if crate::v2::host_fault::is_host_infrastructure_fault(&error) => {
                return Err(error);
            }
            Err(error) if non_retryable_provider_error(&error) => return Err(error),
            Err(error) => {
                let message = error.to_string();
                let provider = provider_error(&error);
                let retryable = provider.is_some_and(|fault| fault.is_operational())
                    || transient_live_agent_error_for_request(&agent_request, &message);
                if retryable {
                    // Typed metadata distinguishes a server hint from a
                    // client's default wait. Display text cannot override it.
                    let delay = retry_delay(attempt).max(
                        provider
                            .and_then(|fault| fault.retry_after_hint())
                            .map(Duration::from_secs)
                            .unwrap_or_default(),
                    );
                    let operational = provider.map_or_else(
                        || archon_llm::provider::LlmError::Http(message.clone()).is_operational(),
                        |fault| fault.is_operational(),
                    );
                    // Refund failed-attempt time only. Existing no-progress
                    // time survives an outage, and partial/idle streams earn
                    // no credit. Inner provider opens pause all session clocks.
                    let paused = if operational {
                        clock.as_ref().map(|clock| {
                            if !clock.provider_accounted() {
                                clock.refund(clock.elapsed().saturating_sub(spent));
                            }
                            clock.pause_for_slot()
                        })
                    } else {
                        None
                    };
                    // Notifications also carry control checks, heartbeat and
                    // trace bookkeeping. Only the log line may be throttled.
                    on_retry(attempt).await?;
                    if retry_is_visible(attempt) {
                        tracing::warn!(
                            component = "workflow_agent",
                            attempt,
                            next_delay_ms = delay.as_millis() as u64,
                            error_class = retry_error_class(&message),
                            "retrying after transient provider failure"
                        );
                    }
                    tokio::time::sleep(delay).await;
                    drop(paused);
                    attempt = attempt.saturating_add(1);
                    continue;
                }
                return Err(error);
            }
        }
    }
}

fn provider_error(error: &WorkflowError) -> Option<&archon_llm::provider::LlmError> {
    let root: &(dyn std::error::Error + 'static) = match error {
        WorkflowError::Port(source) => source.as_ref(),
        other => other,
    };
    std::iter::successors(Some(root), |error| error.source())
        .find_map(|source| source.downcast_ref::<archon_llm::provider::LlmError>())
}

fn non_retryable_provider_error(error: &WorkflowError) -> bool {
    provider_error(error).is_some_and(|fault| fault.is_permanent())
}

/// Shared saturating backoff after a failed attempt, numbered from one.
pub fn retry_delay(attempt: usize) -> Duration {
    let exponent = u32::try_from(attempt.saturating_sub(1)).unwrap_or(u32::MAX);
    INITIAL_AGENT_RETRY_DELAY
        .saturating_mul(2u32.saturating_pow(exponent))
        .min(MAX_AGENT_RETRY_DELAY)
}

fn retry_is_visible(attempt: usize) -> bool {
    attempt.is_power_of_two() || attempt >= 8
}

fn retry_error_class(message: &str) -> &'static str {
    let text = message.to_ascii_lowercase();
    if text.contains("rate limit") || text.contains("rate limited") || text.contains("429") {
        "rate_limit"
    } else if text.contains("overloaded") {
        "overload"
    } else if text.contains("connection") || text.contains("socket") || text.contains("request") {
        "connection"
    } else {
        "transient"
    }
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
    // Issue-213 C2d: the runner's own stop, never a provider blink.
    // Issue 364: a host's completed no-progress decision is not a provider
    // failure to replay. Ordinary connection faults are retried here.
    if crate::error::is_no_progress_stop_text(message) {
        return false;
    }
    let text = message.to_ascii_lowercase();
    if let Some(status) = archon_llm::provider_fault::status_field(&text) {
        return matches!(status, 408 | 429 | 500..=599);
    }
    if non_retryable_provider_fault(&text) {
        return false;
    }
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
        "server overloaded",
    ]
    .iter()
    .any(|needle| text.contains(needle))
}

fn non_retryable_provider_fault(text: &str) -> bool {
    if [
        "context window exceeded",
        "context-too-large",
        "context too large",
    ]
    .iter()
    .any(|marker| text.contains(marker))
    {
        return true;
    }

    false
}

/// The pipeline-type guard this used to carry is gone with the concrete
/// request type: every call routed through here is a workflow stage, so the
/// check was always true and only the auto-background flag ever decided.
pub fn transient_live_agent_error_for_request(request: &WorkflowAgentCall, message: &str) -> bool {
    if crate::error::is_host_call_timeout_text(message) {
        return false;
    }
    if request.disable_auto_background && foreground_subagent_timeout_or_cancel(message) {
        return false;
    }
    transient_live_agent_error(message)
}

/// A host cut — the wall clock, or inactivity — and a cancellation are the
/// host's own decisions, not provider blinks: re-asking them here would
/// restart the session the host just ended.
fn foreground_subagent_timeout_or_cancel(message: &str) -> bool {
    let text = message.to_ascii_lowercase();
    crate::error::is_inactivity_timeout_text(message)
        || text.contains("subagent timed out")
        || text.contains("subagent cancelled")
        || text.contains("subagent auto-backgrounded")
}

#[cfg(test)]
#[path = "llm_retry_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "llm_retry_fault_tests.rs"]
mod fault_tests;

#[cfg(test)]
#[path = "llm_retry_connection_tests.rs"]
mod connection_tests;
