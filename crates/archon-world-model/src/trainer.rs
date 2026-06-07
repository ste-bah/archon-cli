//! Dynamic trainer scheduling gates.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::BackendKind;
use crate::embedding::WorldEmbeddingAdapter;
use crate::registry::ModelRegistry;
use crate::storage::WorldModelStore;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DynamicTrainerPolicy {
    pub min_throttle_ms: u64,
    pub idle_required_ms: u64,
    pub battery_suspend_below_percent: u8,
    pub max_runtime_ms: u64,
}

impl Default for DynamicTrainerPolicy {
    fn default() -> Self {
        Self {
            min_throttle_ms: 3_600_000,
            idle_required_ms: 300_000,
            battery_suspend_below_percent: 30,
            max_runtime_ms: 300_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrainerRuntimeSnapshot {
    pub last_activity_age_ms: u64,
    pub last_training_age_ms: Option<u64>,
    pub battery_percent: Option<u8>,
    pub unplugged: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrainerDecisionReason {
    Ready,
    RecentActivity,
    Throttled,
    LowBattery,
    NoTrigger,
    NotEnoughRows,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrainerDecision {
    pub should_train: bool,
    pub reason: TrainerDecisionReason,
    pub max_runtime_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DynamicTrainerTriggerPolicy {
    pub trigger_new_rows: u64,
    pub trigger_surprises: u64,
    pub trigger_corrections: u64,
    pub trigger_elapsed_ms: u64,
    pub first_run_threshold: u64,
}

impl Default for DynamicTrainerTriggerPolicy {
    fn default() -> Self {
        Self {
            trigger_new_rows: 100,
            trigger_surprises: 5,
            trigger_corrections: 3,
            trigger_elapsed_ms: 21_600_000,
            first_run_threshold: 300,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DynamicTrainerTriggerSnapshot {
    pub total_rows: u64,
    pub candidate_count: u64,
    pub new_rows_since_training: u64,
    pub surprises_since_training: u64,
    pub corrections_since_training: u64,
    pub elapsed_since_training_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrainerTriggerReason {
    FirstRunThreshold,
    NewRows,
    Surprises,
    Corrections,
    Elapsed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DynamicTrainerRunReport {
    pub decision: TrainerDecision,
    pub trigger: Option<TrainerTriggerReason>,
    pub rows_loaded: usize,
    pub examples: usize,
    pub candidate_id: Option<String>,
    pub checkpoint_path: Option<PathBuf>,
    pub training_mean_cosine_error: Option<f32>,
}

pub fn evaluate_dynamic_trainer(
    policy: DynamicTrainerPolicy,
    snapshot: TrainerRuntimeSnapshot,
) -> TrainerDecision {
    if snapshot.last_activity_age_ms < policy.idle_required_ms {
        return decision(false, TrainerDecisionReason::RecentActivity, policy);
    }

    if snapshot
        .last_training_age_ms
        .is_some_and(|age| age < policy.min_throttle_ms)
    {
        return decision(false, TrainerDecisionReason::Throttled, policy);
    }

    if snapshot.unplugged
        && snapshot
            .battery_percent
            .is_some_and(|level| level < policy.battery_suspend_below_percent)
    {
        return decision(false, TrainerDecisionReason::LowBattery, policy);
    }

    decision(true, TrainerDecisionReason::Ready, policy)
}

pub fn evaluate_trainer_trigger(
    policy: DynamicTrainerTriggerPolicy,
    snapshot: DynamicTrainerTriggerSnapshot,
) -> Option<TrainerTriggerReason> {
    if snapshot.candidate_count == 0 && snapshot.total_rows >= policy.first_run_threshold {
        return Some(TrainerTriggerReason::FirstRunThreshold);
    }
    if snapshot.new_rows_since_training >= policy.trigger_new_rows {
        return Some(TrainerTriggerReason::NewRows);
    }
    if snapshot.surprises_since_training >= policy.trigger_surprises {
        return Some(TrainerTriggerReason::Surprises);
    }
    if snapshot.corrections_since_training >= policy.trigger_corrections {
        return Some(TrainerTriggerReason::Corrections);
    }
    if snapshot
        .elapsed_since_training_ms
        .is_some_and(|elapsed| elapsed >= policy.trigger_elapsed_ms)
    {
        return Some(TrainerTriggerReason::Elapsed);
    }
    None
}

pub fn run_dynamic_training_once(
    root: &Path,
    state_dim: usize,
    backend: BackendKind,
    allow_cpu_fallback: bool,
    adapter: &dyn WorldEmbeddingAdapter,
    policy: DynamicTrainerPolicy,
    trigger_policy: DynamicTrainerTriggerPolicy,
    runtime: TrainerRuntimeSnapshot,
    triggers: DynamicTrainerTriggerSnapshot,
) -> Result<DynamicTrainerRunReport> {
    run_dynamic_training_once_controlled(
        root,
        state_dim,
        backend,
        allow_cpu_fallback,
        adapter,
        policy,
        trigger_policy,
        runtime,
        triggers,
        None,
    )
}

pub fn run_dynamic_training_once_controlled(
    root: &Path,
    state_dim: usize,
    backend: BackendKind,
    allow_cpu_fallback: bool,
    adapter: &dyn WorldEmbeddingAdapter,
    policy: DynamicTrainerPolicy,
    trigger_policy: DynamicTrainerTriggerPolicy,
    runtime: TrainerRuntimeSnapshot,
    triggers: DynamicTrainerTriggerSnapshot,
    should_stop: Option<&dyn Fn() -> bool>,
) -> Result<DynamicTrainerRunReport> {
    let mut decision = evaluate_dynamic_trainer(policy, runtime);
    let trigger = evaluate_trainer_trigger(trigger_policy, triggers);
    if !decision.should_train {
        return Ok(report(decision, trigger, 0, 0, None, None, None));
    }
    if trigger.is_none() {
        decision = decision_with_reason(policy, TrainerDecisionReason::NoTrigger);
        return Ok(report(decision, None, 0, 0, None, None, None));
    }

    check_training_stop(should_stop, "world-model row load")?;
    let rows = WorldModelStore::open(root)?.load_rows()?;
    check_training_stop(should_stop, "world-model example build")?;
    let examples =
        crate::train::examples_from_rows_with_adapter_controlled(&rows, adapter, should_stop)?;
    if examples.is_empty() {
        decision = decision_with_reason(policy, TrainerDecisionReason::NotEnoughRows);
        return Ok(report(decision, trigger, rows.len(), 0, None, None, None));
    }

    check_training_stop(should_stop, "world-model candidate train")?;
    let started = std::time::Instant::now();
    let (model, outcome) = crate::train::train_candidate_with_backend_or_cpu_fallback(
        state_dim,
        &examples,
        backend,
        allow_cpu_fallback,
    )?;
    if started.elapsed().as_millis() > u128::from(policy.max_runtime_ms) {
        bail!("world-model training exceeded max_runtime_ms");
    }
    check_training_stop(should_stop, "world-model candidate write")?;
    let registry = ModelRegistry::open(root)?;
    let path = registry.write_candidate(&model, &outcome)?;
    Ok(report(
        decision,
        trigger,
        rows.len(),
        examples.len(),
        Some(model.metadata.model_id),
        Some(path),
        Some(outcome.training_mean_cosine_error),
    ))
}

fn check_training_stop(should_stop: Option<&dyn Fn() -> bool>, stage: &str) -> Result<()> {
    if should_stop.is_some_and(|check| check()) {
        bail!("world-model training stopped or timed out during {stage}");
    }
    Ok(())
}

fn decision(
    should_train: bool,
    reason: TrainerDecisionReason,
    policy: DynamicTrainerPolicy,
) -> TrainerDecision {
    TrainerDecision {
        should_train,
        reason,
        max_runtime_ms: policy.max_runtime_ms,
    }
}

fn decision_with_reason(
    policy: DynamicTrainerPolicy,
    reason: TrainerDecisionReason,
) -> TrainerDecision {
    decision(false, reason, policy)
}

fn report(
    decision: TrainerDecision,
    trigger: Option<TrainerTriggerReason>,
    rows_loaded: usize,
    examples: usize,
    candidate_id: Option<String>,
    checkpoint_path: Option<PathBuf>,
    training_mean_cosine_error: Option<f32>,
) -> DynamicTrainerRunReport {
    DynamicTrainerRunReport {
        decision,
        trigger,
        rows_loaded,
        examples,
        candidate_id,
        checkpoint_path,
        training_mean_cosine_error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idle_snapshot() -> TrainerRuntimeSnapshot {
        TrainerRuntimeSnapshot {
            last_activity_age_ms: 600_000,
            last_training_age_ms: None,
            battery_percent: Some(80),
            unplugged: false,
        }
    }

    #[test]
    fn trainer_suspends_while_session_is_active() {
        let snapshot = TrainerRuntimeSnapshot {
            last_activity_age_ms: 60_000,
            ..idle_snapshot()
        };

        let decision = evaluate_dynamic_trainer(DynamicTrainerPolicy::default(), snapshot);

        assert!(!decision.should_train);
        assert_eq!(decision.reason, TrainerDecisionReason::RecentActivity);
    }

    #[test]
    fn trainer_suspends_on_low_unplugged_battery() {
        let snapshot = TrainerRuntimeSnapshot {
            battery_percent: Some(20),
            unplugged: true,
            ..idle_snapshot()
        };

        let decision = evaluate_dynamic_trainer(DynamicTrainerPolicy::default(), snapshot);

        assert!(!decision.should_train);
        assert_eq!(decision.reason, TrainerDecisionReason::LowBattery);
    }

    #[test]
    fn trainer_runs_when_idle_and_safe() {
        let decision = evaluate_dynamic_trainer(DynamicTrainerPolicy::default(), idle_snapshot());

        assert!(decision.should_train);
        assert_eq!(decision.max_runtime_ms, 300_000);
    }

    #[test]
    fn trigger_policy_detects_first_run_and_new_rows() {
        let policy = DynamicTrainerTriggerPolicy::default();
        let first = DynamicTrainerTriggerSnapshot {
            total_rows: 300,
            candidate_count: 0,
            new_rows_since_training: 0,
            surprises_since_training: 0,
            corrections_since_training: 0,
            elapsed_since_training_ms: None,
        };
        let new_rows = DynamicTrainerTriggerSnapshot {
            candidate_count: 1,
            total_rows: 400,
            new_rows_since_training: 100,
            ..first
        };

        assert_eq!(
            evaluate_trainer_trigger(policy, first),
            Some(TrainerTriggerReason::FirstRunThreshold)
        );
        assert_eq!(
            evaluate_trainer_trigger(policy, new_rows),
            Some(TrainerTriggerReason::NewRows)
        );
    }

    #[test]
    fn dynamic_training_tick_writes_candidate_when_triggered() {
        use crate::embedding::DeterministicHashEmbeddingAdapter;
        use crate::schema::{WorldActionKind, WorldTraceRow};

        let temp = tempfile::tempdir().unwrap();
        let store = WorldModelStore::open(temp.path()).unwrap();
        let mut first = WorldTraceRow::new("s1", WorldActionKind::ToolCall).with_row_id("r1");
        first.redacted_excerpt = Some("run tests".into());
        let mut second = WorldTraceRow::new("s1", WorldActionKind::Verification).with_row_id("r2");
        second.redacted_excerpt = Some("tests passed".into());
        store.persist_rows(&[first, second]).unwrap();
        let adapter = DeterministicHashEmbeddingAdapter::new(4).unwrap();

        let run = run_dynamic_training_once(
            temp.path(),
            4,
            BackendKind::Cpu,
            true,
            &adapter,
            DynamicTrainerPolicy::default(),
            DynamicTrainerTriggerPolicy::default(),
            idle_snapshot(),
            DynamicTrainerTriggerSnapshot {
                total_rows: 300,
                candidate_count: 0,
                new_rows_since_training: 0,
                surprises_since_training: 0,
                corrections_since_training: 0,
                elapsed_since_training_ms: None,
            },
        )
        .unwrap();

        assert!(run.candidate_id.is_some());
        assert!(run.checkpoint_path.unwrap().exists());
    }
}
