//! Release-gate behaviour over derived cognitive metrics.
//!
//! The load-bearing case is `pooled_passes_but_one_failing_cohort_fails_the_gate`:
//! `metrics/derive.rs` reports the pooled cohort *alongside* the segments
//! precisely so a promotion cannot be decided on the aggregate, and a gate that
//! only read the pooled number would satisfy the issue while defeating that.

use archon_cognitive::metrics::{
    CognitiveMetricEvent, GateOutcome, MetricCohort, MetricEventKind, MetricEventStore,
    ReleaseGateVerdict, derive_snapshot, evaluate_release_gate, metric_thresholds,
    thresholds_match_definition_version, unknown_threshold_metrics,
};
use archon_cognitive::{CognitiveTick, SituationKind};
use archon_policy::CognitivePolicy;
use chrono::{DateTime, Duration, TimeZone, Utc};
use cozo::{DbInstance, ScriptMutability};

const COVERAGE_METRIC: &str = "verified_success_label_coverage";

fn at(minute: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap() + Duration::minutes(minute)
}

fn cohort(task_class: &str) -> MetricCohort {
    MetricCohort::new(task_class, "model-a", "policy-1")
}

/// One `world_label_materialized` observation. `outcome_status` is what
/// `verified_success_label_coverage` counts: `passed`/`failed` are deterministic
/// labels, anything else is not.
fn label_event(id: &str, task_class: &str, outcome: &str) -> CognitiveMetricEvent {
    CognitiveMetricEvent::new(
        id,
        "world_label_materialized",
        MetricEventKind::WorldLabelMaterialized,
        "window-1",
        cohort(task_class),
        at(1),
    )
    .with_session("session-1", 1)
    .with_identity("action_attempt_id", format!("attempt-{id}"))
    .with_identity("prediction_id", format!("prediction-{id}"))
    .with_identity("verification_id", format!("verification-{id}"))
    .with_identity("label_definition_version", "1")
    .with_outcome(outcome)
}

/// `healthy` deterministic labels then `unknown` unlabelled ones.
fn label_events(task_class: &str, healthy: usize, unknown: usize) -> Vec<CognitiveMetricEvent> {
    (0..healthy)
        .map(|index| label_event(&format!("{task_class}-ok-{index}"), task_class, "passed"))
        .chain((0..unknown).map(|index| {
            label_event(
                &format!("{task_class}-unknown-{index}"),
                task_class,
                "unknown",
            )
        }))
        .collect()
}

fn coverage_threshold_min_samples() -> usize {
    metric_thresholds()
        .iter()
        .find(|threshold| threshold.metric_name == COVERAGE_METRIC)
        .expect("coverage threshold is declared")
        .min_sample_count
}

/// THE case. Pooled coverage is 44/60 = 0.733, comfortably above the 0.5
/// floor, while the `research` segment sits at 4/20 = 0.200. A gate reading the
/// aggregate would promote; this one must not.
#[test]
fn pooled_passes_but_one_failing_cohort_fails_the_gate() {
    let mut events = label_events("code_change", 40, 0);
    events.extend(label_events("research", 4, 16));

    let snapshot = derive_snapshot(None, &events);
    let pooled = snapshot.pooled(COVERAGE_METRIC).expect("pooled coverage");
    assert!(
        pooled.value.expect("pooled value") > 0.5,
        "pooled coverage must pass for this test to mean anything: {pooled:?}"
    );

    let report = evaluate_release_gate(&snapshot);

    assert_eq!(report.verdict, ReleaseGateVerdict::Failed);
    assert_eq!(
        report.failing_segments(),
        vec!["research/model-a/policy-1".to_string()],
        "the failing segment must be named, and the healthy one must not be"
    );
    let summary = report.failure_summary();
    assert_eq!(summary.len(), 1, "{summary:?}");
    assert!(
        summary[0].contains("research/model-a/policy-1")
            && summary[0].contains("observed=0.2000")
            && summary[0].contains(">= 0.5000"),
        "{summary:?}"
    );

    // The pooled cohort is still reported, and still passes: the gate does not
    // hide the aggregate, it just refuses to decide on it.
    let pooled_check = report
        .checks
        .iter()
        .find(|check| check.cohort.is_pooled())
        .expect("pooled check reported");
    assert_eq!(pooled_check.outcome, GateOutcome::Passed);
    assert_eq!(report.segments_evaluated, 2);
}

/// A segment below the declared sample floor is reported as unjudged, never
/// rounded into a pass — and never used to fail a release either.
#[test]
fn a_thin_cohort_is_insufficient_evidence_rather_than_a_verdict() {
    let thin = coverage_threshold_min_samples() - 1;
    let events = label_events("research", 0, thin);

    let report = evaluate_release_gate(&derive_snapshot(None, &events));

    assert_eq!(report.verdict, ReleaseGateVerdict::NotEvaluated);
    assert_eq!(report.segments_evaluated, 0);
    // The distinction this test draws is between "reported as unjudged" and
    // "silently absent", and `.all()` over an empty `checks` cannot tell them
    // apart — a gate that emitted no checks at all would satisfy it, alongside
    // `segments_evaluated == 0`. The thin cohort has to appear in the report.
    assert!(
        !report.checks.is_empty(),
        "the thin cohort must be reported as unjudged, not omitted from the \
         report altogether"
    );
    assert!(
        report
            .checks
            .iter()
            .all(|check| check.outcome == GateOutcome::InsufficientEvidence),
        "{:?}",
        report.checks
    );
}

#[test]
fn a_snapshot_with_no_metrics_is_not_a_pass() {
    let report = evaluate_release_gate(&derive_snapshot(None, &[]));

    assert_eq!(report.verdict, ReleaseGateVerdict::NotEvaluated);
    assert!(!report.blocks_promotion());
    assert!(report.checks.is_empty());
}

#[test]
fn a_healthy_cohort_passes_the_gate() {
    let events = label_events("code_change", 40, 0);

    let report = evaluate_release_gate(&derive_snapshot(None, &events));

    assert_eq!(report.verdict, ReleaseGateVerdict::Passed);
    assert!(!report.blocks_promotion());
    assert_eq!(report.segments_evaluated, 1);
}

/// A threshold naming a metric no definition derives would silently never fire.
#[test]
fn every_threshold_names_a_defined_metric_at_the_live_definition_version() {
    assert_eq!(unknown_threshold_metrics(), Vec::<&str>::new());
    assert!(
        thresholds_match_definition_version(),
        "metric definitions changed without revisiting the thresholds derived from them"
    );
}

// ---------------------------------------------------------------------------
// Live call site: `CognitiveTick` consults the gate before generating the
// behaviour-change proposals the governed apply path promotes.
// ---------------------------------------------------------------------------

fn policy() -> CognitivePolicy {
    CognitivePolicy {
        enabled: true,
        allow_autonomous_tick: true,
        allow_autonomous_low_risk_apply: true,
        allow_self_model_updates: true,
        max_autonomous_risk: "Low".into(),
        ..CognitivePolicy::default()
    }
}

fn insert_proposable_reflection(db: &DbInstance, id: &str) {
    let script = format!(
        "?[reflection_id, session_id, turn_number, decision_id, situation_kind, attempted, worked, failed, outcome, lesson, should_propose, proposed_rule_id, created_at] <- \
         [['{id}', 'session-1', 1, 'decision-{id}', '{}', 'attempted', '', 'failed', 'failure', 'answer format should include compact evidence', true, '', '{}']]
         :put cognitive_reflections {{ reflection_id => session_id, turn_number, decision_id, situation_kind, attempted, worked, failed, outcome, lesson, should_propose, proposed_rule_id, created_at }}",
        SituationKind::Greeting.as_str(),
        Utc::now().to_rfc3339()
    );
    db.run_script(&script, Default::default(), ScriptMutability::Mutable)
        .unwrap();
}

fn record(db: &DbInstance, dir: &std::path::Path, events: Vec<CognitiveMetricEvent>) {
    let store = MetricEventStore::new(db, dir).unwrap();
    for event in &events {
        store.record(event).unwrap();
    }
}

/// The tick must actually consult the gate: one degraded segment withholds
/// autonomous self-modification even though the pooled figure is healthy.
#[test]
fn tick_withholds_promotion_when_one_cohort_fails_the_gate() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let dir = tempfile::tempdir().unwrap();
    archon_cognitive::ensure_cognitive_schema(&db).unwrap();
    insert_proposable_reflection(&db, "r1");

    let mut events = label_events("code_change", 40, 0);
    events.extend(label_events("research", 4, 16));
    record(&db, dir.path(), events);

    let report = CognitiveTick::new(&db, Some(policy()), dir.path())
        .unwrap()
        .tick()
        .unwrap();

    assert_eq!(report.proposals_generated, 0);
    let blocked: Vec<&String> = report
        .errors
        .iter()
        .filter(|error| error.starts_with("release_gate_blocked:"))
        .collect();
    assert_eq!(blocked.len(), 1, "{:?}", report.errors);
    assert!(
        blocked[0].contains("research/model-a/policy-1"),
        "{blocked:?}"
    );
}

/// Control: the same tick with the degraded segment healthy promotes as before,
/// which is what proves the gate — and not something else — did the blocking.
#[test]
fn tick_promotes_when_every_cohort_passes_the_gate() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let dir = tempfile::tempdir().unwrap();
    archon_cognitive::ensure_cognitive_schema(&db).unwrap();
    insert_proposable_reflection(&db, "r1");

    let mut events = label_events("code_change", 40, 0);
    events.extend(label_events("research", 20, 0));
    record(&db, dir.path(), events);

    let report = CognitiveTick::new(&db, Some(policy()), dir.path())
        .unwrap()
        .tick()
        .unwrap();

    assert_eq!(report.proposals_generated, 1);
    assert!(
        report
            .errors
            .iter()
            .all(|error| !error.starts_with("release_gate_")),
        "{:?}",
        report.errors
    );
}
