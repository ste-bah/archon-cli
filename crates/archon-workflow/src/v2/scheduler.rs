use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use futures_util::future::join_all;
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;

use crate::{WorkflowError, WorkflowResult};

use super::{
    WorkflowV2HostCall, WorkflowV2Result, WorkflowV2Status, WorkflowV2TaskCompletionEvidence,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowV2FanoutItem {
    pub id: String,
    pub role: String,
    pub call: WorkflowV2HostCall,
    #[serde(default)]
    pub input: serde_json::Value,
}

impl WorkflowV2FanoutItem {
    pub fn read_only(
        id: impl Into<String>,
        role: impl Into<String>,
        call: WorkflowV2HostCall,
        input: serde_json::Value,
    ) -> Self {
        Self {
            id: id.into(),
            role: role.into(),
            call,
            input,
        }
    }

    pub fn input_hash(&self) -> String {
        stable_value_hash(&self.input)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowV2SchedulerConfig {
    pub max_parallelism: usize,
    pub absolute_max_parallelism: usize,
    pub role_limits: BTreeMap<String, usize>,
    pub branch_timeout: Option<Duration>,
}

impl Default for WorkflowV2SchedulerConfig {
    fn default() -> Self {
        Self {
            max_parallelism: 8,
            absolute_max_parallelism: 16,
            role_limits: BTreeMap::new(),
            branch_timeout: None,
        }
    }
}

impl WorkflowV2SchedulerConfig {
    pub fn effective_parallelism(&self) -> usize {
        self.max_parallelism
            .max(1)
            .min(self.absolute_max_parallelism.max(1))
    }

    fn role_limit(&self, role: &str) -> usize {
        self.role_limits
            .get(role)
            .copied()
            .unwrap_or_else(|| self.effective_parallelism())
            .max(1)
    }
}

#[derive(Debug, Clone, Default)]
pub struct WorkflowV2CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl WorkflowV2CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchFailureKind {
    Semantic,
    Contract,
    Safety,
    Execution,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowV2BranchOutcome {
    pub item_id: String,
    pub role: String,
    pub status: WorkflowV2Status,
    pub result: Option<WorkflowV2Result>,
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_kind: Option<BranchFailureKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_input_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub completion_evidence: Vec<WorkflowV2TaskCompletionEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowV2FanoutReport {
    pub outcomes: Vec<WorkflowV2BranchOutcome>,
    pub max_parallelism: usize,
    pub peak_parallelism: usize,
    pub cancelled: bool,
}

impl WorkflowV2FanoutReport {
    pub fn typed_results(&self) -> Vec<&WorkflowV2Result> {
        self.outcomes
            .iter()
            .filter_map(|outcome| outcome.result.as_ref())
            .collect()
    }

    pub fn failed_outcomes(&self) -> Vec<&WorkflowV2BranchOutcome> {
        self.outcomes
            .iter()
            .filter(|outcome| outcome.status == WorkflowV2Status::Failed)
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct WorkflowV2Scheduler {
    config: WorkflowV2SchedulerConfig,
    cancellation: WorkflowV2CancellationToken,
}

impl WorkflowV2Scheduler {
    pub fn new(config: WorkflowV2SchedulerConfig) -> Self {
        Self {
            config,
            cancellation: WorkflowV2CancellationToken::new(),
        }
    }

    pub fn with_cancellation(
        config: WorkflowV2SchedulerConfig,
        cancellation: WorkflowV2CancellationToken,
    ) -> Self {
        Self {
            config,
            cancellation,
        }
    }

    pub fn cancellation_token(&self) -> WorkflowV2CancellationToken {
        self.cancellation.clone()
    }

    pub async fn run_read_only_fanout<F, Fut>(
        &self,
        items: Vec<WorkflowV2FanoutItem>,
        handler: F,
    ) -> WorkflowResult<WorkflowV2FanoutReport>
    where
        F: Fn(WorkflowV2FanoutItem) -> Fut + Sync,
        Fut: Future<Output = WorkflowResult<WorkflowV2Result>>,
    {
        self.run_read_only_fanout_observed(items, |_| Ok(()), handler)
            .await
    }

    pub async fn run_read_only_fanout_observed<F, Fut, O>(
        &self,
        items: Vec<WorkflowV2FanoutItem>,
        observer: O,
        handler: F,
    ) -> WorkflowResult<WorkflowV2FanoutReport>
    where
        F: Fn(WorkflowV2FanoutItem) -> Fut + Sync,
        Fut: Future<Output = WorkflowResult<WorkflowV2Result>>,
        O: Fn(&WorkflowV2BranchOutcome) -> WorkflowResult<()> + Sync,
    {
        reject_write_capable_items(&items)?;
        if items.is_empty() {
            return Err(WorkflowError::SpecInvalid(
                "read-only fanout resolved zero items without typed no-op proof".to_string(),
            ));
        }

        let max_parallelism = self.config.effective_parallelism();
        let global_semaphore = Arc::new(Semaphore::new(max_parallelism));
        let role_semaphores = role_semaphores(&items, &self.config);
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let cancellation = self.cancellation.clone();

        let jobs = items.into_iter().map(|item| {
            let global_semaphore = global_semaphore.clone();
            let role_semaphore = role_semaphores
                .get(&item.role)
                .cloned()
                .unwrap_or_else(|| Arc::new(Semaphore::new(1)));
            let active = active.clone();
            let peak = peak.clone();
            let cancellation = cancellation.clone();
            let handler = &handler;
            let observer = &observer;
            let branch_timeout = self.config.branch_timeout;
            async move {
                let timeout_item = item.clone();
                let branch = run_branch(
                    item,
                    global_semaphore,
                    role_semaphore,
                    active,
                    peak,
                    cancellation,
                    handler,
                );
                let outcome = if let Some(timeout) = branch_timeout {
                    match tokio::time::timeout(timeout, branch).await {
                        Ok(outcome) => outcome?,
                        Err(_) => timed_out_outcome(timeout_item, timeout),
                    }
                } else {
                    branch.await?
                };
                observer(&outcome)?;
                Ok(outcome)
            }
        });

        let outcomes = join_all(jobs)
            .await
            .into_iter()
            .collect::<WorkflowResult<Vec<_>>>()?;
        Ok(WorkflowV2FanoutReport {
            outcomes,
            max_parallelism,
            peak_parallelism: peak.load(Ordering::SeqCst),
            cancelled: self.cancellation.is_cancelled(),
        })
    }
}

async fn run_branch<F, Fut>(
    item: WorkflowV2FanoutItem,
    global_semaphore: Arc<Semaphore>,
    role_semaphore: Arc<Semaphore>,
    active: Arc<AtomicUsize>,
    peak: Arc<AtomicUsize>,
    cancellation: WorkflowV2CancellationToken,
    handler: &F,
) -> WorkflowResult<WorkflowV2BranchOutcome>
where
    F: Fn(WorkflowV2FanoutItem) -> Fut + Sync,
    Fut: Future<Output = WorkflowResult<WorkflowV2Result>>,
{
    if cancellation.is_cancelled() {
        return Ok(cancelled_outcome(item));
    }

    let global_permit = match global_semaphore.acquire_owned().await {
        Ok(permit) => permit,
        Err(err) => {
            return Ok(failed_outcome_with_kind(
                item,
                err.to_string(),
                BranchFailureKind::Execution,
            ));
        }
    };
    let role_permit = match role_semaphore.acquire_owned().await {
        Ok(permit) => permit,
        Err(err) => {
            return Ok(failed_outcome_with_kind(
                item,
                err.to_string(),
                BranchFailureKind::Execution,
            ));
        }
    };

    if cancellation.is_cancelled() {
        return Ok(cancelled_outcome(item));
    }

    let now_active = active.fetch_add(1, Ordering::SeqCst) + 1;
    record_peak(&peak, now_active);
    let result = handler(item.clone()).await;
    active.fetch_sub(1, Ordering::SeqCst);
    drop(role_permit);
    drop(global_permit);

    match result {
        Ok(result) => match result.validate() {
            Ok(()) => {
                let item_input_hash = item.input_hash();
                Ok(WorkflowV2BranchOutcome {
                    item_id: item.id,
                    role: item.role,
                    status: result.status,
                    failure_kind: failure_kind_for_valid_status(result.status),
                    result: Some(result),
                    error: None,
                    item_input_hash: Some(item_input_hash),
                    completion_evidence: Vec::new(),
                })
            }
            Err(err) => Ok(failed_outcome_with_kind(
                item,
                format!("invalid branch result: {err}"),
                BranchFailureKind::Contract,
            )),
        },
        Err(WorkflowError::ControlPaused(message)) => Err(WorkflowError::ControlPaused(message)),
        Err(WorkflowError::ControlCancelled(message)) => {
            Err(WorkflowError::ControlCancelled(message))
        }
        Err(err) => {
            let message = err.to_string();
            let kind = classify_branch_error(&message);
            Ok(failed_outcome_with_kind(item, message, kind))
        }
    }
}

fn reject_write_capable_items(items: &[WorkflowV2FanoutItem]) -> WorkflowResult<()> {
    if let Some(item) = items.iter().find(|item| item.call.write_mode.is_some()) {
        return Err(WorkflowError::PolicyDenied(format!(
            "read-only fanout item '{}' declares write mode; use a safe write-mode scheduler",
            item.id
        )));
    }
    Ok(())
}

fn role_semaphores(
    items: &[WorkflowV2FanoutItem],
    config: &WorkflowV2SchedulerConfig,
) -> BTreeMap<String, Arc<Semaphore>> {
    items
        .iter()
        .map(|item| {
            (
                item.role.clone(),
                Arc::new(Semaphore::new(config.role_limit(&item.role))),
            )
        })
        .collect()
}

fn record_peak(peak: &AtomicUsize, observed: usize) {
    let mut current = peak.load(Ordering::SeqCst);
    while observed > current {
        match peak.compare_exchange(current, observed, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => return,
            Err(next) => current = next,
        }
    }
}

fn failed_outcome_with_kind(
    item: WorkflowV2FanoutItem,
    error: String,
    failure_kind: BranchFailureKind,
) -> WorkflowV2BranchOutcome {
    let item_input_hash = item.input_hash();
    WorkflowV2BranchOutcome {
        item_id: item.id,
        role: item.role,
        status: WorkflowV2Status::Failed,
        result: None,
        error: Some(error),
        failure_kind: Some(failure_kind),
        item_input_hash: Some(item_input_hash),
        completion_evidence: Vec::new(),
    }
}

fn cancelled_outcome(item: WorkflowV2FanoutItem) -> WorkflowV2BranchOutcome {
    let item_input_hash = item.input_hash();
    WorkflowV2BranchOutcome {
        item_id: item.id,
        role: item.role,
        status: WorkflowV2Status::Cancelled,
        result: None,
        error: Some("fanout cancelled before branch execution".to_string()),
        failure_kind: Some(BranchFailureKind::Execution),
        item_input_hash: Some(item_input_hash),
        completion_evidence: Vec::new(),
    }
}

fn timed_out_outcome(item: WorkflowV2FanoutItem, timeout: Duration) -> WorkflowV2BranchOutcome {
    failed_outcome_with_kind(
        item,
        format!(
            "fanout branch timed out after {} second(s)",
            timeout.as_secs()
        ),
        BranchFailureKind::Execution,
    )
}

fn failure_kind_for_valid_status(status: WorkflowV2Status) -> Option<BranchFailureKind> {
    match status {
        WorkflowV2Status::Failed | WorkflowV2Status::Blocked | WorkflowV2Status::NeedsReview => {
            Some(BranchFailureKind::Semantic)
        }
        WorkflowV2Status::Cancelled => Some(BranchFailureKind::Execution),
        _ => None,
    }
}

fn classify_branch_error(error: &str) -> BranchFailureKind {
    let lower = error.to_ascii_lowercase();
    if lower.contains("changed files outside")
        || lower.contains("outside declared target_files")
        || lower.contains("outside declared ownership")
        || lower.contains("read-only")
        || lower.contains("declares no target ownership")
        || lower.contains("patch apply")
        || lower.contains("ownership")
    {
        return BranchFailureKind::Safety;
    }
    if lower.contains("agent transport failed")
        || lower.contains("tool execution failed")
        || lower.contains("process failed")
        || lower.contains("timed out")
        || lower.contains("rate limit")
        || lower.contains("cancelled")
    {
        return BranchFailureKind::Execution;
    }
    BranchFailureKind::Contract
}

/// The one content hash for workflow JSON payloads.
///
/// Serialization alone is already canonical here: `serde_json`'s
/// `preserve_order` feature is NOT enabled in this workspace, so `Map` is a
/// `BTreeMap` and object keys always serialize in sorted order. Arrays are
/// serialized in their given order — deliberately. Array position is
/// semantically load-bearing in the payloads this hashes (fan-out branch
/// identity falls back to the item index, reducer arguments are a positional
/// tuple, declared verifier commands run in array order fail-fast), so a hash
/// that sorted array elements would give `[a, b]` and `[b, a]` the same key and
/// let a reordered run replay the wrong cached result.
pub fn stable_value_hash(value: &serde_json::Value) -> String {
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    blake3::hash(&bytes).to_hex().to_string()
}
