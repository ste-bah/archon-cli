use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio::time::{Duration, sleep};

use crate::error::{WorkflowError, WorkflowResult};
use crate::policy::WorkflowPolicy;
use crate::run::{ArtifactRef, ItemState, StageStatus, WorkflowRun};
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

pub(crate) fn width(run: &WorkflowRun, policy: &WorkflowPolicy) -> usize {
    run.spec.max_parallelism.min(policy.max_parallelism).max(1) as usize
}

/// Fan-out width honoring an optional stage-level `max_parallelism` override.
///
/// The stage value (when present) caps the width but is itself clamped by the
/// run-level `spec.max_parallelism` and the policy ceiling, so a stage can only
/// request LESS parallelism than the run/policy allow, never more. When the
/// stage does not set `max_parallelism` the run-level width is used.
pub(crate) fn stage_width(run: &WorkflowRun, policy: &WorkflowPolicy, stage: &StageSpec) -> usize {
    let base = width(run, policy);
    match stage.max_parallelism {
        Some(stage_cap) => base.min((stage_cap.max(1)) as usize),
        None => base,
    }
}

/// Fan-out semaphore width clamped to the runner's concurrency capacity.
///
/// The spec/policy `max_parallelism` sets the desired width, but a live runner
/// backed by a bounded subagent manager rejects work beyond its hard cap. When
/// the runner advertises a finite capacity we clamp to it so overflow items
/// wait on the semaphore instead of being hard-rejected as terminal failures.
pub(crate) fn runner_clamped_width(
    run: &WorkflowRun,
    policy: &WorkflowPolicy,
    stage: &StageSpec,
    runner_capacity: Option<usize>,
) -> usize {
    let base = stage_width(run, policy, stage);
    match runner_capacity {
        Some(cap) => base.min(cap.max(1)),
        None => base,
    }
}

pub(crate) fn accepted_item_cached(run: &WorkflowRun, item_id: &str) -> bool {
    run.items
        .get(item_id)
        .is_some_and(|item| item.status == StageStatus::Accepted && item.artifact.is_some())
}

pub(crate) fn record_item(
    run: &mut WorkflowRun,
    stage: &StageSpec,
    item_id: String,
    status: StageStatus,
    artifact: Option<ArtifactRef>,
    error: Option<String>,
) {
    run.items.insert(
        item_id.clone(),
        ItemState {
            id: item_id,
            stage_id: stage.id.clone(),
            status,
            artifact,
            error,
        },
    );
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
    let text = err.to_string().to_ascii_lowercase();
    text.contains("429")
        || text.contains("rate limit")
        || text.contains("timeout")
        || text.contains("temporar")
}
