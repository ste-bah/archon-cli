//! Dynamic trainer scheduling gates.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::BackendKind;
use crate::registry::ModelRegistry;
use crate::replay::{ReplayPlan, ReplayPolicy, ReplaySummary};
use crate::representation::{TraceWindowBuilder, WorldRepresentationAdapter};
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

pub type TrainerStopCallback<'a> = dyn Fn() -> bool + 'a;

pub struct DynamicTrainingRequest<'a> {
    pub root: &'a Path,
    pub state_dim: usize,
    pub backend: BackendKind,
    pub allow_cpu_fallback: bool,
    pub adapter: &'a dyn WorldRepresentationAdapter,
    pub context_rows: usize,
    pub policy: DynamicTrainerPolicy,
    pub trigger_policy: DynamicTrainerTriggerPolicy,
    pub runtime: TrainerRuntimeSnapshot,
    pub triggers: DynamicTrainerTriggerSnapshot,
    /// Surprise-weighted replay policy.
    ///
    /// Its plan is computed on every run that reaches example construction and
    /// reported on [`DynamicTrainerRunReport::replay`]; it changes the example
    /// set only when `prioritized_enabled` is set, which is `false` by default.
    pub replay: ReplayPolicy,
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
    /// Replay plan for this run, whenever examples were built.
    ///
    /// Present even when the plan was not applied: an unapplied plan is the
    /// shadow evidence â€” pool size, held-out size, surprise coverage, decile
    /// concentration, importance-weight range â€” that has to exist before
    /// prioritisation may be turned on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replay: Option<ReplaySummary>,
}

impl DynamicTrainerRunReport {
    fn with_replay(mut self, summary: ReplaySummary) -> Self {
        self.replay = Some(summary);
        self
    }
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
    request: &DynamicTrainingRequest<'_>,
) -> Result<DynamicTrainerRunReport> {
    run_dynamic_training_once_controlled(request, None)
}

pub fn run_dynamic_training_once_controlled(
    request: &DynamicTrainingRequest<'_>,
    should_stop: Option<&TrainerStopCallback>,
) -> Result<DynamicTrainerRunReport> {
    let DynamicTrainingRequest {
        root,
        state_dim,
        backend,
        allow_cpu_fallback,
        adapter,
        context_rows,
        policy,
        trigger_policy,
        runtime,
        triggers,
        replay,
    } = request;
    let mut decision = evaluate_dynamic_trainer(*policy, *runtime);
    let trigger = evaluate_trainer_trigger(*trigger_policy, *triggers);
    if !decision.should_train {
        return Ok(report(decision, trigger, 0, 0, None, None, None));
    }
    if trigger.is_none() {
        decision = decision_with_reason(*policy, TrainerDecisionReason::NoTrigger);
        return Ok(report(decision, None, 0, 0, None, None, None));
    }

    check_training_stop(should_stop, "world-model row load")?;
    let rows = WorldModelStore::open(root)?.load_verified_training_rows()?;
    check_training_stop(should_stop, "world-model example build")?;
    let examples = crate::train::examples_from_rows_with_representation_adapter_controlled(
        &rows,
        *adapter,
        *context_rows,
        should_stop,
    )?;
    if examples.is_empty() {
        decision = decision_with_reason(*policy, TrainerDecisionReason::NotEnoughRows);
        return Ok(report(decision, trigger, rows.len(), 0, None, None, None));
    }

    check_training_stop(should_stop, "world-model replay plan")?;
    let (examples, replay_summary) = apply_replay_plan(&rows, root, *replay, examples)?;

    check_training_stop(should_stop, "world-model candidate train")?;
    let started = std::time::Instant::now();
    let (model, outcome) = crate::train::train_candidate_with_backend_or_cpu_fallback(
        *state_dim,
        &examples,
        *backend,
        *allow_cpu_fallback,
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
        // The count of examples actually trained on, which the replay summary
        // beside it explains: equal to the built count unless a plan applied.
        examples.len(),
        Some(model.metadata.model_id),
        Some(path),
        Some(outcome.training_mean_cosine_error),
    )
    .with_replay(replay_summary))
}

fn check_training_stop(should_stop: Option<&TrainerStopCallback>, stage: &str) -> Result<()> {
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
        replay: None,
    }
}

/// Build the replay plan for a run and return the examples training will use.
///
/// The plan is always computed: it is the shadow evidence for W6, and a
/// diagnostic that only runs once someone enables it is a diagnostic nobody can
/// trust. `examples` is narrowed only when the plan reports itself applied.
fn apply_replay_plan(
    rows: &[crate::schema::WorldTraceRow],
    root: &Path,
    policy: ReplayPolicy,
    examples: Vec<crate::model::LatentTransitionExample>,
) -> Result<(Vec<crate::model::LatentTransitionExample>, ReplaySummary)> {
    // Same horizon `examples_from_rows_with_representation_adapter_controlled`
    // uses, from the same shared window scan, so position `i` here is the
    // transition behind `examples[i]`. The length check inside `plan_replay`
    // refuses the plan outright if that ever stops holding.
    let keys = TraceWindowBuilder::new(rows).adjacent_transition_keys(1)?;
    // Outcomes carry the latent surprise the guarded-action loop recorded; a
    // corpus with none simply produces a plan that declines to apply.
    let outcomes = crate::guardrail::load_guardrail_outcomes(root).unwrap_or_default();
    let surprise = crate::replay::surprise_by_row_id(rows, &outcomes);
    let plan: ReplayPlan = crate::replay::plan_replay(&keys, &surprise, policy, examples.len());
    let summary = plan.summary.clone();
    if !plan.applied() {
        return Ok((examples, summary));
    }
    let selected = plan
        .selected_indices()
        .into_iter()
        .map(|index| examples[index].clone())
        .collect();
    Ok((selected, summary))
}

#[cfg(test)]
#[path = "trainer/00_tests.rs"]
mod tests;
