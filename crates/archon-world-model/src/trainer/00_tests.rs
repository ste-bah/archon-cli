use crate::embedding::DeterministicHashEmbeddingAdapter;
use crate::guardrail::{
    GuardedActionKind, GuardrailFinalStatus, RuntimeTaskClass, WorldGuardedAction,
    WorldGuardrailOutcome, append_guarded_action, append_guardrail_outcome,
};
use crate::integration::WorldAdvisorSurface;
use crate::replay::{ReplaySkipReason, is_held_out};
use crate::representation::GenericEmbeddingRepresentationAdapter;
use crate::schema::{WorldActionKind, WorldTraceRow};

use super::*;

fn idle_snapshot() -> TrainerRuntimeSnapshot {
    TrainerRuntimeSnapshot {
        last_activity_age_ms: 600_000,
        last_training_age_ms: None,
        battery_percent: Some(80),
        unplugged: false,
    }
}

fn first_run_triggers() -> DynamicTrainerTriggerSnapshot {
    DynamicTrainerTriggerSnapshot {
        total_rows: 300,
        candidate_count: 0,
        new_rows_since_training: 0,
        surprises_since_training: 0,
        corrections_since_training: 0,
        elapsed_since_training_ms: None,
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
    let first = first_run_triggers();
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
    let temp = tempfile::tempdir().unwrap();
    let store = WorldModelStore::open(temp.path()).unwrap();
    let mut first = WorldTraceRow::new("s1", WorldActionKind::ToolCall).with_row_id("r1");
    first.redacted_excerpt = Some("run tests".into());
    let mut second = WorldTraceRow::new("s1", WorldActionKind::Verification).with_row_id("r2");
    second.redacted_excerpt = Some("tests passed".into());
    store.persist_rows(&[first, second]).unwrap();
    let adapter = GenericEmbeddingRepresentationAdapter::new(Box::new(
        DeterministicHashEmbeddingAdapter::new(4).unwrap(),
    ));

    let request = DynamicTrainingRequest {
        root: temp.path(),
        state_dim: 4,
        backend: BackendKind::Cpu,
        allow_cpu_fallback: true,
        adapter: &adapter,
        context_rows: 1,
        policy: DynamicTrainerPolicy::default(),
        trigger_policy: DynamicTrainerTriggerPolicy::default(),
        runtime: idle_snapshot(),
        triggers: first_run_triggers(),
        replay: ReplayPolicy::default(),
    };
    let run = run_dynamic_training_once(&request).unwrap();

    assert!(run.candidate_id.is_some());
    assert!(run.checkpoint_path.unwrap().exists());
}

/// The plan is built on a real training run, not only in unit tests, and it
/// declines to apply on a corpus that never recorded a surprise. Default
/// policy: what the model trains on is unchanged.
#[test]
fn dynamic_training_reports_an_unapplied_replay_plan_by_default() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorldModelStore::open(temp.path()).unwrap();
    let rows: Vec<WorldTraceRow> = (0..6)
        .map(|index| {
            let mut row = WorldTraceRow::new("s1", WorldActionKind::ToolCall)
                .with_row_id(format!("r{index}"));
            row.redacted_excerpt = Some(format!("step {index}"));
            row
        })
        .collect();
    let built = rows.len() - 1;
    store.persist_rows(&rows).unwrap();
    let adapter = GenericEmbeddingRepresentationAdapter::new(Box::new(
        DeterministicHashEmbeddingAdapter::new(4).unwrap(),
    ));

    let request = DynamicTrainingRequest {
        root: temp.path(),
        state_dim: 4,
        backend: BackendKind::Cpu,
        allow_cpu_fallback: true,
        adapter: &adapter,
        context_rows: 1,
        policy: DynamicTrainerPolicy::default(),
        trigger_policy: DynamicTrainerTriggerPolicy::default(),
        runtime: idle_snapshot(),
        triggers: first_run_triggers(),
        replay: ReplayPolicy::default(),
    };
    let run = run_dynamic_training_once(&request).unwrap();

    let replay = run.replay.expect("replay plan computed on a real run");
    assert!(!replay.applied);
    // No surprise was ever recorded for this corpus, and the flag is off;
    // whichever of those the plan names first, it must not be applied.
    assert!(matches!(
        replay.skip_reason,
        Some(ReplaySkipReason::NoSurpriseSignal | ReplaySkipReason::EmptyPool)
    ));
    assert_eq!(replay.transitions, built);
    // Trained on every built example: the plan changed nothing.
    assert_eq!(run.examples, built);
}

/// Sessions chosen by the split itself, so the corpus deterministically has both
/// a training and a held-out side.
fn split_sessions(training: usize, held_out: usize) -> (Vec<String>, Vec<String>) {
    let mut train = Vec::new();
    let mut held = Vec::new();
    for index in 0..2_000 {
        let session = format!("session-{index:04}");
        if is_held_out(&session, 0.2, 1) {
            if held.len() < held_out {
                held.push(session);
            }
        } else if train.len() < training {
            train.push(session);
        }
        if train.len() == training && held.len() == held_out {
            break;
        }
    }
    (train, held)
}

/// End to end with the flag on: real rows, real guarded actions, real recorded
/// latent surprise, through `run_dynamic_training_once`. The example set is
/// narrowed to the batch, the held-out sessions are excluded, and the summary
/// carries the correction that makes the change auditable.
#[test]
fn enabled_replay_narrows_a_real_training_run_and_excludes_held_out_sessions() {
    const ROWS_PER_SESSION: usize = 4;
    let (training_sessions, held_out_sessions) = split_sessions(4, 1);
    let temp = tempfile::tempdir().unwrap();
    let store = WorldModelStore::open(temp.path()).unwrap();

    let mut rows = Vec::new();
    let mut surprise_step = 0.0_f32;
    for session in training_sessions.iter().chain(held_out_sessions.iter()) {
        for index in 0..ROWS_PER_SESSION {
            let action = WorldGuardedAction::new(
                session,
                WorldAdvisorSurface::InteractiveSession,
                GuardedActionKind::ShellCommand,
                "goal",
                "summary",
            );
            let mut row = WorldTraceRow::new(session, WorldActionKind::ToolCall)
                .with_row_id(format!("{session}-r{index}"))
                .with_action_attempt_id(action.action_id.clone());
            row.redacted_excerpt = Some(format!("{session} step {index}"));
            surprise_step += 0.03;
            let outcome = WorldGuardrailOutcome {
                action_id: action.action_id.clone(),
                task_class: RuntimeTaskClass::CodingChange,
                final_status: GuardrailFinalStatus::CompletedVerified,
                latent_surprise: Some(surprise_step),
                ..WorldGuardrailOutcome::default()
            };
            append_guarded_action(temp.path(), &action).unwrap();
            append_guardrail_outcome(temp.path(), &outcome).unwrap();
            rows.push(row);
        }
    }
    store.persist_rows(&rows).unwrap();

    let adapter = GenericEmbeddingRepresentationAdapter::new(Box::new(
        DeterministicHashEmbeddingAdapter::new(4).unwrap(),
    ));
    let batch_size = 6;
    let request = DynamicTrainingRequest {
        root: temp.path(),
        state_dim: 4,
        backend: BackendKind::Cpu,
        allow_cpu_fallback: true,
        adapter: &adapter,
        context_rows: 1,
        policy: DynamicTrainerPolicy::default(),
        trigger_policy: DynamicTrainerTriggerPolicy::default(),
        runtime: idle_snapshot(),
        triggers: first_run_triggers(),
        replay: ReplayPolicy {
            prioritized_enabled: true,
            batch_size,
            ..ReplayPolicy::default()
        },
    };

    let run = run_dynamic_training_once(&request).unwrap();
    let replay = run.replay.expect("replay summary");

    // 5 sessions x (4 rows - 1) adjacent transitions.
    let transitions_per_session = ROWS_PER_SESSION - 1;
    assert_eq!(replay.transitions, 5 * transitions_per_session);
    assert_eq!(replay.pool, 4 * transitions_per_session);
    assert_eq!(replay.held_out, transitions_per_session);
    assert_eq!(replay.held_out_sessions, 1);
    assert_eq!(replay.with_surprise, replay.pool);

    assert!(replay.applied, "flag on and surprise present: must apply");
    assert_eq!(replay.skip_reason, None);
    assert_eq!(replay.selected, batch_size);
    // The point of the change: fewer examples than were built, drawn from the
    // training partition only.
    assert_eq!(run.examples, batch_size);
    assert!(run.examples < replay.transitions);
    assert!(run.candidate_id.is_some());

    // The correction that keeps a prioritised batch from silently redefining
    // the training distribution is recorded, and bounded.
    assert!(replay.min_importance_weight > 0.0);
    assert!(replay.max_importance_weight <= 2.0 + 1e-4);
    assert!(replay.max_decile_share <= 0.40 + 1e-4);
    assert_eq!(replay.priority_version, crate::replay::PRIORITY_VERSION);
}
