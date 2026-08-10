use chrono::{DateTime, Duration, TimeZone, Utc};
use cozo::DbInstance;

use archon_cognitive::CognitiveInspection;
use archon_cognitive::metrics::{
    CognitiveMetricEvent, EvaluationWindow, MetricCohort, MetricEventKind, MetricEventStore,
    derive_snapshot,
};

fn at(minute: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap() + Duration::minutes(minute)
}

fn window() -> EvaluationWindow {
    EvaluationWindow::new("window-1", at(0), at(60))
}

fn code_change() -> MetricCohort {
    MetricCohort::new("code_change", "model-a", "policy-1")
}

fn research() -> MetricCohort {
    MetricCohort::new("research", "model-a", "policy-1")
}

fn rule_event(
    id: &str,
    operation: &str,
    cohort: MetricCohort,
    at: DateTime<Utc>,
) -> CognitiveMetricEvent {
    CognitiveMetricEvent::new(
        id,
        "rule_lifecycle",
        MetricEventKind::RuleLifecycleObserved,
        "window-1",
        cohort,
        at,
    )
    .with_session("session-1", 1)
    .with_identity("rule_id", "rule-1")
    .with_identity("rule_operation", operation)
}

fn correction_event(id: &str, turn: u64, label: &str) -> CognitiveMetricEvent {
    CognitiveMetricEvent::new(
        id,
        "correction_classified",
        MetricEventKind::CorrectionClassified,
        "window-1",
        code_change(),
        at(1),
    )
    .with_session("session-1", turn)
    .with_identity("correction_id", format!("correction-{turn}"))
    .with_identity("predicted_label", label)
    .with_identity("ground_truth_label", label)
    .with_identity("abstained", "false")
}

fn prompt_event(id: &str, stale: f64, injected: f64) -> CognitiveMetricEvent {
    CognitiveMetricEvent::new(
        id,
        "prompt_rules_composed",
        MetricEventKind::PromptRulesComposed,
        "window-1",
        code_change(),
        at(3),
    )
    .with_session("session-1", 3)
    .with_identity("prompt_snapshot_id", format!("prompt-{id}"))
    .with_identity("rule_state_snapshot_id", "rule-state-1")
    .with_identity("ordered_injected_rule_ids", "[\"rule-1\"]")
    .with_identity("stale_definition_version", "1")
    .with_ratio(stale, injected)
}

fn prediction_event(id: &str, predicted: f64, outcome: &str) -> CognitiveMetricEvent {
    CognitiveMetricEvent::new(
        id,
        "self_model_prediction_evaluated",
        MetricEventKind::SelfModelPredictionEvaluated,
        "window-1",
        code_change(),
        at(4),
    )
    .with_session("session-1", 4)
    .with_identity("self_model_prediction_id", format!("prediction-{id}"))
    .with_identity("self_model_fact_id", "fact-1")
    .with_identity("self_model_dimension", "tool_success")
    .with_identity("self_model_backed", "true")
    .with_identity("verification_id", format!("verification-{id}"))
    .with_value(predicted)
    .with_outcome(outcome)
}

fn surprise_event(id: &str, value: f64) -> CognitiveMetricEvent {
    CognitiveMetricEvent::new(
        id,
        "surprise_observed",
        MetricEventKind::SurpriseObserved,
        "window-1",
        code_change(),
        at(5),
    )
    .with_session("session-1", 5)
    .with_identity("prediction_id", format!("prediction-{id}"))
    .with_identity("action_attempt_id", format!("attempt-{id}"))
    .with_identity("verification_id", format!("verification-{id}"))
    .with_value(value)
}

#[test]
fn derivation_only_counts_events_inside_the_window() {
    let inside = rule_event("metric-1", "create", code_change(), at(10));
    let boundary = rule_event("metric-2", "create", code_change(), at(60));
    let outside = rule_event("metric-3", "create", code_change(), at(90));

    let snapshot = derive_snapshot(Some(&window()), &[inside, boundary, outside]);

    // The window is half-open, so the event landing exactly on `ended_at`
    // belongs to the next window, not this one.
    assert_eq!(snapshot.event_count, 1);
    assert_eq!(
        snapshot.pooled("rule_create_count").unwrap().value,
        Some(1.0)
    );
}

#[test]
fn events_declaring_a_different_window_are_not_absorbed_by_this_one() {
    let mut foreign = rule_event("metric-1", "create", code_change(), at(10));
    foreign.evaluation_window_id = "window-2".into();

    let snapshot = derive_snapshot(Some(&window()), &[foreign]);

    assert_eq!(snapshot.event_count, 0);
    assert!(snapshot.metrics.is_empty());
}

#[test]
fn metrics_are_reported_per_cohort_and_pooled() {
    let events = vec![
        rule_event("metric-1", "create", code_change(), at(10)),
        rule_event("metric-2", "create", code_change(), at(11)),
        rule_event("metric-3", "create", research(), at(12)),
        rule_event("metric-4", "retire", research(), at(13)),
    ];

    let snapshot = derive_snapshot(Some(&window()), &events);

    assert_eq!(snapshot.cohort_count, 2);
    assert_eq!(
        snapshot
            .metric("rule_create_count", &code_change())
            .unwrap()
            .value,
        Some(2.0)
    );
    assert_eq!(
        snapshot
            .metric("rule_create_count", &research())
            .unwrap()
            .value,
        Some(1.0)
    );
    assert_eq!(
        snapshot.pooled("rule_create_count").unwrap().value,
        Some(3.0)
    );
    // A cohort with no matching events reports nothing rather than a zero.
    assert!(
        snapshot
            .metric("rule_retire_count", &code_change())
            .is_none()
    );
    assert_eq!(
        snapshot
            .metric("rule_retire_count", &research())
            .unwrap()
            .value,
        Some(1.0)
    );
}

#[test]
fn pooled_ratio_uses_the_defined_value_when_the_denominator_is_zero() {
    let events = vec![
        prompt_event("metric-1", 0.0, 0.0),
        prompt_event("metric-2", 0.0, 0.0),
    ];

    let snapshot = derive_snapshot(Some(&window()), &events);
    let share = snapshot.pooled("stale_rule_prompt_share").unwrap();

    // Roadmap version 1 defines the empty-prompt share as 0, not undefined.
    assert_eq!(share.denominator, 0.0);
    assert_eq!(share.value, Some(0.0));
}

#[test]
fn pooled_ratio_aggregates_numerators_and_denominators_not_per_prompt_shares() {
    let events = vec![
        prompt_event("metric-1", 1.0, 1.0),
        prompt_event("metric-2", 1.0, 9.0),
    ];

    let snapshot = derive_snapshot(Some(&window()), &events);
    let share = snapshot.pooled("stale_rule_prompt_share").unwrap();

    // Mean of per-prompt shares would be 0.55; the pooled ratio is 2/10.
    assert_eq!(share.value, Some(0.2));
    assert_eq!(share.sample_count, 2);
}

#[test]
fn brier_score_excludes_outcomes_without_deterministic_verification() {
    let events = vec![
        prediction_event("metric-1", 1.0, "passed"),
        prediction_event("metric-2", 0.0, "failed"),
        prediction_event("metric-3", 0.5, "unknown"),
    ];

    let snapshot = derive_snapshot(Some(&window()), &events);
    let brier = snapshot
        .pooled("self_model_confidence_calibration_error")
        .unwrap();

    assert_eq!(brier.value, Some(0.0));
    assert_eq!(brier.denominator, 2.0);
    // The unknown outcome is still part of the eligible event population; it
    // is excluded from the score, not deleted from the evidence.
    assert_eq!(brier.sample_count, 3);
}

#[test]
fn corrections_per_100_turns_divides_by_distinct_observed_turns() {
    let events = vec![
        correction_event("metric-1", 1, "correction"),
        correction_event("metric-2", 2, "correction"),
        correction_event("metric-3", 3, "not_correction"),
        correction_event("metric-4", 4, "not_correction"),
    ];

    let snapshot = derive_snapshot(Some(&window()), &events);
    let rate = snapshot.pooled("corrections_per_100_turns").unwrap();

    assert_eq!(rate.denominator, 4.0);
    assert_eq!(rate.value, Some(50.0));
    assert_eq!(
        snapshot
            .pooled("correction_classifier_abstention_rate")
            .unwrap()
            .value,
        Some(0.0)
    );
}

#[test]
fn percentiles_use_nearest_rank_over_observed_values() {
    let events: Vec<CognitiveMetricEvent> = (1..=20)
        .map(|index| surprise_event(&format!("metric-{index}"), f64::from(index)))
        .collect();

    let snapshot = derive_snapshot(Some(&window()), &events);

    assert_eq!(
        snapshot.pooled("latent_surprise_mean").unwrap().value,
        Some(10.5)
    );
    assert_eq!(
        snapshot.pooled("latent_surprise_p95").unwrap().value,
        Some(19.0)
    );
}

#[test]
fn snapshot_from_the_store_matches_the_declared_window() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = MetricEventStore::new(&db, dir.path()).unwrap();
    store.declare_window(&window()).unwrap();
    store
        .record(&rule_event("metric-1", "create", code_change(), at(10)))
        .unwrap();
    store
        .record(&rule_event("metric-2", "create", code_change(), at(90)))
        .unwrap();

    // Independent read through a second handle on the same relation.
    let snapshot = MetricEventStore::new(&db, dir.path())
        .unwrap()
        .latest_snapshot()
        .unwrap();

    assert_eq!(
        snapshot
            .evaluation_window
            .as_ref()
            .unwrap()
            .evaluation_window_id,
        "window-1"
    );
    assert_eq!(snapshot.event_count, 1);
    assert_eq!(
        snapshot.pooled("rule_create_count").unwrap().value,
        Some(1.0)
    );
}

#[test]
fn inspection_status_surfaces_the_derived_snapshot() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = MetricEventStore::new(&db, dir.path()).unwrap();
    store.declare_window(&window()).unwrap();
    store
        .record(&rule_event("metric-1", "create", code_change(), at(10)))
        .unwrap();
    store
        .record(&rule_event("metric-2", "reinforce", research(), at(11)))
        .unwrap();

    let status = CognitiveInspection::new(&db, dir.path())
        .unwrap()
        .status()
        .unwrap();

    assert_eq!(status.metric_event_count, 2);
    assert_eq!(status.metrics.cohort_count, 2);
    assert_eq!(
        status.metrics.pooled("rule_create_count").unwrap().value,
        Some(1.0)
    );
    assert_eq!(
        status
            .metrics
            .metric("rule_reinforce_count", &research())
            .unwrap()
            .value,
        Some(1.0)
    );
}

#[test]
fn without_a_declared_window_the_whole_history_is_the_population() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = MetricEventStore::new(&db, dir.path()).unwrap();
    store
        .record(&rule_event("metric-1", "create", code_change(), at(10)))
        .unwrap();
    store
        .record(&rule_event("metric-2", "create", code_change(), at(900)))
        .unwrap();

    let snapshot = store.latest_snapshot().unwrap();

    assert!(snapshot.evaluation_window.is_none());
    assert_eq!(snapshot.event_count, 2);
}
