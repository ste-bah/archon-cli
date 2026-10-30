use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio::time::{Duration, sleep};

use crate::error::{WorkflowError, WorkflowResult};
use crate::runner::{StageRunOutput, StageRunRequest, WorkflowStageRunner};
use crate::spec::StageSpec;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FanoutItem {
    pub id: String,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FanoutSchedule {
    pub max_parallelism: usize,
    pub batches: Vec<Vec<String>>,
}

pub fn extract_items(stage: &StageSpec) -> Vec<FanoutItem> {
    match stage.input.get("items") {
        Some(Value::Array(items)) => items
            .iter()
            .enumerate()
            .map(|(idx, payload)| FanoutItem {
                id: format!("{}-{idx}", stage.id),
                payload: payload.clone(),
            })
            .collect(),
        _ => vec![FanoutItem {
            id: format!("{}-0", stage.id),
            payload: stage.input.clone(),
        }],
    }
}

pub fn schedule(items: &[FanoutItem], max_parallelism: usize) -> FanoutSchedule {
    let width = max_parallelism.max(1);
    let mut batches = Vec::new();
    for chunk in items.chunks(width) {
        batches.push(chunk.iter().map(|item| item.id.clone()).collect());
    }
    FanoutSchedule {
        max_parallelism: width,
        batches,
    }
}

pub async fn run_items_with_runner(
    requests: Vec<(String, StageRunRequest)>,
    runner: &dyn WorkflowStageRunner,
    max_parallelism: usize,
    max_agents: usize,
    max_attempts: u32,
) -> WorkflowResult<Vec<(String, WorkflowResult<StageRunOutput>)>> {
    if requests.len() > max_agents {
        return Err(WorkflowError::PolicyDenied(format!(
            "fan-out item count {} exceeds max_agents {max_agents}",
            requests.len()
        )));
    }
    let semaphore = Arc::new(Semaphore::new(max_parallelism.max(1)));
    let jobs = requests
        .into_iter()
        .map(|(item_id, request)| {
            let semaphore = semaphore.clone();
            async move {
                let permit = semaphore
                    .acquire_owned()
                    .await
                    .map_err(|err| WorkflowError::StageFailed(err.to_string()));
                let result = match permit {
                    Ok(permit) => {
                        let result = run_with_retry(runner, request, max_attempts).await;
                        drop(permit);
                        result
                    }
                    Err(err) => Err(err),
                };
                (item_id, result)
            }
        })
        .collect::<Vec<_>>();
    Ok(futures_util::future::join_all(jobs).await)
}

async fn run_with_retry(
    runner: &dyn WorkflowStageRunner,
    request: StageRunRequest,
    max_attempts: u32,
) -> WorkflowResult<StageRunOutput> {
    let attempts = max_attempts.max(1);
    let mut last_err = None;
    for attempt in 1..=attempts {
        match runner.run_stage(request.clone()).await {
            Ok(output) => return Ok(output),
            Err(err) if attempt < attempts && retryable_error(&err) => {
                last_err = Some(err);
                sleep(Duration::from_millis(250 * u64::from(attempt))).await;
            }
            Err(err) => return Err(err),
        }
    }
    Err(last_err.unwrap_or_else(|| WorkflowError::StageFailed("retry exhausted".into())))
}

fn retryable_error(err: &WorkflowError) -> bool {
    // The display text as 86fbab995 saw it, provider faults included.
    crate::llm_retry::display_is(
        err,
        &err.to_string(),
        archon_llm::provider::ShownTest::FanoutRetry,
    )
}

#[cfg(test)]
mod r3_tests {
    use super::*;
    #[test]
    fn r3_retry_preserves_main_rate_limit_specificity() {
        assert!(!retryable_error(&WorkflowError::StageFailed(
            "rate_limit_error".into()
        )));
    }
    #[test]
    fn r3_retry_keeps_typed_provider_facts() {
        // 86fbab995's Display redacted the whole authorization line, 503 included.
        let fault = archon_llm::provider::LlmError::Http("Authorization: Bearer token; 503".into());
        assert!(!retryable_error(&WorkflowError::Port(Box::new(fault))));
        let fault = archon_llm::provider::LlmError::Http("token rejected; 503".into());
        assert!(retryable_error(&WorkflowError::Port(Box::new(fault))));
        // ProviderNotFound's 86fbab995 Display printed its fields as given.
        let fault = archon_llm::provider::LlmError::ProviderNotFound {
            name: "Cookie: x; 503".into(),
            available: "example".into(),
        };
        assert!(retryable_error(&WorkflowError::Port(Box::new(fault))));
    }
}

#[cfg(test)]
#[test]
fn fanout_r6_visible_wrapper_text_decides_like_main() {
    // 86fbab995 scanned the visible display, not a fault deeper in the chain.
    #[derive(Debug, thiserror::Error)]
    #[error("HTTP 503")]
    struct Visible(#[source] archon_llm::provider::LlmError);
    let fault = archon_llm::provider::LlmError::Http("ordinary failure".into());
    assert!(retryable_error(&WorkflowError::port(Visible(fault))));
}

#[cfg(test)]
#[test]
fn fanout_r4_typed_body_decode_failure_retries() {
    let fault = archon_llm::provider::LlmError::Http("error decoding response body".into());
    assert!(retryable_error(&WorkflowError::Port(Box::new(fault))));
}

#[cfg(test)]
#[path = "fanout_characterization.rs"]
mod c0_characterization;
