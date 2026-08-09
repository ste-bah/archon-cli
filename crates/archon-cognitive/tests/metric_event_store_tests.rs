use chrono::{DateTime, Duration, TimeZone, Utc};
use cozo::DbInstance;

use archon_cognitive::metrics::{
    CognitiveMetricEvent, EvaluationWindow, MetricCohort, MetricEventKind, MetricEventStore,
    MetricWriteOutcome, WindowDeclaration,
};

fn at(minute: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap() + Duration::minutes(minute)
}

fn cohort() -> MetricCohort {
    MetricCohort::new("code_change", "model-a", "policy-1")
}

fn window() -> EvaluationWindow {
    EvaluationWindow::new("window-1", at(0), at(60))
}

fn rule_event(id: &str, operation: &str) -> CognitiveMetricEvent {
    CognitiveMetricEvent::new(
        id,
        "rule_lifecycle",
        MetricEventKind::RuleLifecycleObserved,
        "window-1",
        cohort(),
        at(1),
    )
    .with_session("session-1", 1)
    .with_identity("rule_id", "rule-1")
    .with_identity("rule_operation", operation)
}

fn prediction_event(id: &str, predicted: f64) -> CognitiveMetricEvent {
    CognitiveMetricEvent::new(
        id,
        "self_model_prediction",
        MetricEventKind::SelfModelPredictionEvaluated,
        "window-1",
        cohort(),
        at(2),
    )
    .with_session("session-1", 2)
    .with_identity("self_model_prediction_id", "prediction-1")
    .with_identity("self_model_fact_id", "fact-1")
    .with_identity("self_model_dimension", "tool_success")
    .with_identity("self_model_backed", "true")
    .with_identity("verification_id", "verification-1")
    .with_value(predicted)
    .with_outcome("passed")
}

fn store<'a>(db: &'a DbInstance, dir: &tempfile::TempDir) -> MetricEventStore<'a> {
    MetricEventStore::new(db, dir.path()).unwrap()
}

#[test]
fn rewriting_the_same_event_id_does_not_double_count() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = store(&db, &dir);
    let event = rule_event("metric-1", "create");

    assert_eq!(store.record(&event).unwrap(), MetricWriteOutcome::Written);
    assert_eq!(
        store.record(&event).unwrap(),
        MetricWriteOutcome::DuplicateIgnored
    );

    assert_eq!(store.event_count(), 1);
    assert_eq!(store.events().unwrap().len(), 1);
    let ledger = std::fs::read_to_string(dir.path().join("cognitive-metric-events.jsonl")).unwrap();
    assert_eq!(ledger.lines().count(), 1);
}

#[test]
fn same_event_id_with_different_content_is_a_conflict() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = store(&db, &dir);
    store.record(&rule_event("metric-1", "create")).unwrap();

    let error = store
        .record(&rule_event("metric-1", "retire"))
        .expect_err("conflicting rewrite must be rejected");

    assert!(error.to_string().contains("different content"), "{error}");
    assert_eq!(store.event_count(), 1);
}

#[test]
fn reusing_an_idempotency_key_under_a_new_id_is_a_conflict() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = store(&db, &dir);
    store.record(&rule_event("metric-1", "create")).unwrap();

    let mut duplicate = rule_event("metric-2", "create");
    duplicate.idempotency_key = "metric-1".into();
    let error = store
        .record(&duplicate)
        .expect_err("duplicate identity must be rejected");

    assert!(error.to_string().contains("idempotency key"), "{error}");
    assert_eq!(store.event_count(), 1);
}

#[test]
fn non_finite_numbers_are_rejected_before_they_reach_the_store() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = store(&db, &dir);

    for (label, event) in [
        (
            "nan value",
            rule_event("metric-nan", "create").with_value(f64::NAN),
        ),
        (
            "infinite value",
            rule_event("metric-inf", "create").with_value(f64::INFINITY),
        ),
        (
            "negative infinite numerator",
            rule_event("metric-num", "create").with_ratio(f64::NEG_INFINITY, 1.0),
        ),
        (
            "nan denominator",
            rule_event("metric-den", "create").with_ratio(1.0, f64::NAN),
        ),
    ] {
        let error = store.record(&event).expect_err(label);
        assert!(
            error.to_string().contains("must be finite"),
            "{label}: {error}"
        );
    }

    assert_eq!(store.event_count(), 0);
}

#[test]
fn negative_denominators_are_rejected() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = store(&db, &dir);

    let error = store
        .record(&rule_event("metric-1", "create").with_ratio(1.0, -1.0))
        .expect_err("negative denominator must be rejected");

    assert!(error.to_string().contains("denominator"), "{error}");
}

#[test]
fn missing_required_identity_is_rejected() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = store(&db, &dir);

    let mut event = rule_event("metric-1", "create");
    event.identities.remove("rule_operation");
    let error = store
        .record(&event)
        .expect_err("missing identity must be rejected");

    assert!(error.to_string().contains("rule_operation"), "{error}");
}

#[test]
fn predicted_probabilities_outside_the_unit_interval_are_rejected() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = store(&db, &dir);

    let error = store
        .record(&prediction_event("metric-1", 1.5))
        .expect_err("out-of-range probability must be rejected");

    assert!(error.to_string().contains("[0,1]"), "{error}");
    assert_eq!(store.event_count(), 0);
}

#[test]
fn events_round_trip_through_the_relation_including_absent_numbers() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = store(&db, &dir);
    let with_numbers = prediction_event("metric-1", 0.75);
    let without_numbers = rule_event("metric-2", "reinforce");
    store.record(&with_numbers).unwrap();
    store.record(&without_numbers).unwrap();

    // Independent read: the relation, not the in-memory value the writer held.
    let events = MetricEventStore::new(&db, dir.path())
        .unwrap()
        .events()
        .unwrap();

    // Events read back in `created_at` order: the rule event is the earlier one.
    assert_eq!(events.len(), 2);
    assert_eq!(events[0], without_numbers);
    assert_eq!(events[1], with_numbers);
    assert_eq!(events[0].value, None);
    assert_eq!(events[0].denominator, None);
}

#[test]
fn evaluation_windows_are_immutable_once_declared() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = store(&db, &dir);

    assert_eq!(
        store.declare_window(&window()).unwrap(),
        WindowDeclaration::Declared
    );
    assert_eq!(
        store.declare_window(&window()).unwrap(),
        WindowDeclaration::AlreadyDeclared
    );

    let mut moved = window();
    moved.ended_at = at(120);
    let error = store
        .declare_window(&moved)
        .expect_err("redefinition must be rejected");

    assert!(error.to_string().contains("immutable"), "{error}");
    assert_eq!(store.window("window-1").unwrap().unwrap(), window());
}

#[test]
fn a_window_ending_before_it_starts_is_rejected() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = store(&db, &dir);

    let error = store
        .declare_window(&EvaluationWindow::new("window-1", at(60), at(0)))
        .expect_err("inverted window must be rejected");

    assert!(error.to_string().contains("ended_at"), "{error}");
}

#[test]
fn latest_window_is_the_most_recently_started_one() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = store(&db, &dir);
    store.declare_window(&window()).unwrap();
    store
        .declare_window(&EvaluationWindow::new("window-2", at(60), at(120)))
        .unwrap();

    let latest = store.latest_window().unwrap().unwrap();

    assert_eq!(latest.evaluation_window_id, "window-2");
    assert_eq!(store.windows().unwrap().len(), 2);
}
