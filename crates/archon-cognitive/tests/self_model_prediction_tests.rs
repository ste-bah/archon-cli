//! Issue #80(a): the self-model predicts *before* it acts, and is graded after.

use archon_cognitive::self_model::prediction::{
    SELF_MODEL_CALIBRATION_METRIC, SelfModelPredictor, TurnEvidence, TurnVerification,
};
use archon_cognitive::{MetricEventStore, evaluate_release_gate};
use archon_policy::CognitivePolicy;
use cozo::{DbInstance, ScriptMutability};

fn db() -> DbInstance {
    let db = DbInstance::new("mem", "", "").unwrap();
    archon_cognitive::ensure_cognitive_schema(&db).unwrap();
    db
}

fn write_trust_fact(db: &DbInstance, domain: &str, confidence: f64, evidence: i64) {
    let script = format!(
        "?[fact_id, domain, fact_kind, statement, confidence, evidence_count, last_seen_at, expires_at, created_at] <- \
         [['domain_trust:{domain}', '{domain}', 'domain_trust', 'measured', {confidence}, {evidence}, '2026-01-01T00:00:00Z', '', '2026-01-01T00:00:00Z']]
         :put self_model_facts {{ fact_id => domain, fact_kind, statement, confidence, evidence_count, last_seen_at, expires_at, created_at }}"
    );
    db.run_script(&script, Default::default(), ScriptMutability::Mutable)
        .unwrap();
}

fn stored_probability(db: &DbInstance, prediction_id: &str) -> Option<f64> {
    let script = format!(
        "?[predicted_success_probability] := *self_model_predictions{{prediction_id: '{prediction_id}', predicted_success_probability}}"
    );
    db.run_script(&script, Default::default(), ScriptMutability::Immutable)
        .unwrap()
        .rows
        .first()
        .and_then(|row| row[0].get_float())
}

fn policy() -> Option<CognitivePolicy> {
    Some(CognitivePolicy {
        enabled: true,
        ..CognitivePolicy::default()
    })
}

// ── the deterministic label ──────────────────────────────────

/// The label may only come from evidence that is actually deterministic. A turn
/// that completed while executing nothing verified nothing, and coercing that to
/// a pass is the "unverified completion is success" mistake W5 exists to stop.
#[test]
fn only_executed_tool_evidence_produces_a_deterministic_verdict() {
    let cases = [
        (
            TurnEvidence {
                tool_calls: 2,
                tool_failures: 0,
                completed: true,
            },
            TurnVerification::Passed,
        ),
        (
            TurnEvidence {
                tool_calls: 2,
                tool_failures: 1,
                completed: true,
            },
            TurnVerification::Failed,
        ),
        (
            TurnEvidence {
                tool_calls: 2,
                tool_failures: 0,
                completed: false,
            },
            TurnVerification::Failed,
        ),
        (
            TurnEvidence {
                tool_calls: 0,
                tool_failures: 0,
                completed: true,
            },
            TurnVerification::Unknown,
        ),
    ];

    for (evidence, expected) in cases {
        assert_eq!(evidence.verdict(), expected, "{evidence:?}");
    }
    assert!(!TurnVerification::Unknown.is_deterministic());
    assert_eq!(TurnVerification::Unknown.outcome_status(), "unknown");
}

// ── prediction identity ──────────────────────────────────────

/// The population is "self-model-backed turns". A domain the self-model has
/// never measured must not enter it behind a default probability.
#[test]
fn a_domain_with_no_fact_makes_no_prediction() {
    let db = db();
    let dir = tempfile::tempdir().unwrap();
    let predictor = SelfModelPredictor::new(&db, dir.path(), policy()).unwrap();

    assert!(predictor.predict("s1", 1, "coding").unwrap().is_none());
    assert!(
        predictor
            .resolve(
                "s1",
                1,
                TurnEvidence {
                    tool_calls: 1,
                    tool_failures: 0,
                    completed: true
                },
                "test-model",
            )
            .unwrap()
            .is_none()
    );
    assert_eq!(
        MetricEventStore::new(&db, dir.path())
            .unwrap()
            .event_count(),
        0
    );
}

/// The point of the pre-action ordering: resolution attaches a verification and
/// must not touch the probability it is grading.
#[test]
fn resolution_grades_the_pre_action_probability_without_rewriting_it() {
    let db = db();
    let dir = tempfile::tempdir().unwrap();
    write_trust_fact(&db, "coding", 0.8, 12);
    let predictor = SelfModelPredictor::new(&db, dir.path(), policy()).unwrap();

    let prediction = predictor.predict("s1", 3, "coding").unwrap().expect("fact");
    assert!((prediction.predicted_success_probability - 0.8).abs() < 1e-6);
    let stored = stored_probability(&db, &prediction.prediction_id).expect("pending row");
    assert!((stored - 0.8).abs() < 1e-6, "{stored}");

    // The fact moves after the prediction was made, exactly as a self-model
    // refresh would move it mid-session.
    write_trust_fact(&db, "coding", 0.1, 40);

    let resolved = predictor
        .resolve(
            "s1",
            3,
            TurnEvidence {
                tool_calls: 4,
                tool_failures: 1,
                completed: true,
            },
            "test-model",
        )
        .unwrap()
        .expect("pending prediction");

    assert_eq!(resolved.verification, TurnVerification::Failed);
    assert!((resolved.prediction.predicted_success_probability - 0.8).abs() < 1e-6);
    let after = stored_probability(&db, &prediction.prediction_id).expect("resolved row");
    assert!((after - stored).abs() < f64::EPSILON, "{after} != {stored}");
    assert!(resolved.metric_recorded);

    // Resolved once: the pending row is consumed, so a retried join cannot
    // double-count the same turn.
    assert!(
        predictor
            .resolve(
                "s1",
                3,
                TurnEvidence {
                    tool_calls: 4,
                    tool_failures: 1,
                    completed: true
                },
                "test-model",
            )
            .unwrap()
            .is_none()
    );
}

/// An inconclusive turn still resolves its row — otherwise it would be re-graded
/// forever — but contributes nothing to the Brier population.
#[test]
fn an_unverifiable_turn_resolves_without_entering_the_calibration_population() {
    let db = db();
    let dir = tempfile::tempdir().unwrap();
    write_trust_fact(&db, "research", 0.6, 5);
    let predictor = SelfModelPredictor::new(&db, dir.path(), policy()).unwrap();
    predictor
        .predict("s1", 1, "research")
        .unwrap()
        .expect("fact");

    let resolved = predictor
        .resolve(
            "s1",
            1,
            TurnEvidence {
                tool_calls: 0,
                tool_failures: 0,
                completed: true,
            },
            "test-model",
        )
        .unwrap()
        .expect("pending prediction");

    assert_eq!(resolved.verification, TurnVerification::Unknown);
    assert!(!resolved.metric_recorded);
    assert_eq!(
        MetricEventStore::new(&db, dir.path())
            .unwrap()
            .event_count(),
        0
    );
}

// ── what the gate now sees ───────────────────────────────────

/// The whole reason this emitter exists: `self_model_confidence_calibration_error`
/// was a declared release threshold with no events behind it, so the gate could
/// never judge it. It can now.
#[test]
fn emitted_predictions_feed_the_calibration_release_gate() {
    let db = db();
    let dir = tempfile::tempdir().unwrap();
    // A badly calibrated self-model: certain of success, wrong every time.
    write_trust_fact(&db, "coding", 0.95, 30);
    let predictor = SelfModelPredictor::new(&db, dir.path(), policy()).unwrap();
    for turn in 1..=25 {
        predictor
            .predict("s1", turn, "coding")
            .unwrap()
            .expect("fact");
        predictor
            .resolve(
                "s1",
                turn,
                TurnEvidence {
                    tool_calls: 1,
                    tool_failures: 1,
                    completed: true,
                },
                "test-model",
            )
            .unwrap()
            .expect("pending prediction");
    }

    let snapshot = MetricEventStore::new(&db, dir.path())
        .unwrap()
        .latest_snapshot()
        .unwrap();
    let brier = snapshot
        .pooled(SELF_MODEL_CALIBRATION_METRIC)
        .expect("the calibration metric now derives from real events");
    assert_eq!(brier.sample_count, 25);
    // (0.95 - 0)^2 for every turn.
    assert!((brier.value.unwrap() - 0.9025).abs() < 1e-6, "{brier:?}");

    let gate = evaluate_release_gate(&snapshot);
    assert!(
        gate.blocks_promotion(),
        "a self-model this miscalibrated must withhold promotion"
    );
    assert!(
        gate.failure_summary()
            .iter()
            .any(|line| line.contains(SELF_MODEL_CALIBRATION_METRIC)),
        "{:?}",
        gate.failure_summary()
    );
}

/// A calibrated self-model clears the same bound, so the gate is measuring the
/// model rather than the fact that events exist.
#[test]
fn a_calibrated_self_model_passes_the_gate() {
    let db = db();
    let dir = tempfile::tempdir().unwrap();
    write_trust_fact(&db, "coding", 0.8, 30);
    let predictor = SelfModelPredictor::new(&db, dir.path(), policy()).unwrap();
    for turn in 1..=25 {
        predictor
            .predict("s1", turn, "coding")
            .unwrap()
            .expect("fact");
        // Four passes for every failure: an 0.8 prediction is about right.
        let failures = u32::from(turn % 5 == 0);
        predictor
            .resolve(
                "s1",
                turn,
                TurnEvidence {
                    tool_calls: 1,
                    tool_failures: failures,
                    completed: true,
                },
                "test-model",
            )
            .unwrap()
            .expect("pending prediction");
    }

    let snapshot = MetricEventStore::new(&db, dir.path())
        .unwrap()
        .latest_snapshot()
        .unwrap();
    let brier = snapshot.pooled(SELF_MODEL_CALIBRATION_METRIC).unwrap();
    assert!(brier.value.unwrap() < 0.25, "{brier:?}");
    assert!(!evaluate_release_gate(&snapshot).blocks_promotion());
}
