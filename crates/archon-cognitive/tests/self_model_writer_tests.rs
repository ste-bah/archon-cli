use archon_cognitive::SituationKind;
use archon_cognitive::self_model::{MAX_CONFIDENCE_DRIFT, SelfModelStore, SelfModelWriter};
use archon_policy::CognitivePolicy;
use chrono::Utc;
use cozo::{DbInstance, ScriptMutability};

fn policy(allow: bool) -> CognitivePolicy {
    CognitivePolicy {
        enabled: true,
        allow_self_model_updates: allow,
        ..CognitivePolicy::default()
    }
}

fn insert_reflection(db: &DbInstance, id: &str, kind: SituationKind, outcome: &str) {
    let script = format!(
        "?[reflection_id, session_id, turn_number, decision_id, situation_kind, attempted, worked, failed, outcome, lesson, should_propose, proposed_rule_id, created_at] <- \
         [['{id}', 'session-1', 1, 'decision-{id}', '{}', '', '', '', '{outcome}', 'lesson-{id}', false, '', '{}']]
         :put cognitive_reflections {{ reflection_id => session_id, turn_number, decision_id, situation_kind, attempted, worked, failed, outcome, lesson, should_propose, proposed_rule_id, created_at }}",
        kind.as_str(),
        Utc::now().to_rfc3339()
    );
    db.run_script(&script, Default::default(), ScriptMutability::Mutable)
        .unwrap();
}

fn trust_confidence(db: &DbInstance, domain: &str) -> Option<f32> {
    let script = format!(
        "?[confidence] := *self_model_facts{{fact_id: 'domain_trust:{domain}', confidence}}"
    );
    db.run_script(&script, Default::default(), ScriptMutability::Immutable)
        .unwrap()
        .rows
        .first()
        .map(|row| row[0].get_float().unwrap() as f32)
}

#[test]
fn policy_can_withhold_self_model_updates_entirely() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let dir = tempfile::tempdir().unwrap();
    archon_cognitive::ensure_cognitive_schema(&db).unwrap();

    let update = SelfModelWriter::new(&db, dir.path(), Some(policy(false)))
        .refresh_domain_trust()
        .unwrap();

    assert!(
        update.is_none(),
        "withheld must not look like 'nothing to do'"
    );
}

/// Two verified outcomes is below the evidence floor, so the writer must
/// produce no fact at all — not a placeholder at neutral confidence that a
/// reader could not tell apart from a measured result.
#[test]
fn a_domain_below_the_evidence_floor_gets_no_fact_and_says_why() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let dir = tempfile::tempdir().unwrap();
    archon_cognitive::ensure_cognitive_schema(&db).unwrap();
    insert_reflection(&db, "r1", SituationKind::CodeChange, "success");
    insert_reflection(&db, "r2", SituationKind::CodeChange, "failure");

    let update = SelfModelWriter::new(&db, dir.path(), Some(policy(true)))
        .refresh_domain_trust()
        .unwrap()
        .expect("permitted");

    assert_eq!(update.facts_written, 0);
    assert!(!update.changed());
    assert_eq!(
        update.unwritten,
        vec!["insufficient_evidence:coding:2/3".to_string()]
    );
    assert!(trust_confidence(&db, "coding").is_none());
}

/// Non-deterministic outcomes are excluded rather than scored in either
/// direction: folding them in would be inventing a label.
#[test]
fn partial_and_degraded_outcomes_do_not_count_as_evidence() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let dir = tempfile::tempdir().unwrap();
    archon_cognitive::ensure_cognitive_schema(&db).unwrap();
    for (id, outcome) in [
        ("r1", "success"),
        ("r2", "partial_success"),
        ("r3", "degraded"),
    ] {
        insert_reflection(&db, id, SituationKind::CodeChange, outcome);
    }

    let update = SelfModelWriter::new(&db, dir.path(), Some(policy(true)))
        .refresh_domain_trust()
        .unwrap()
        .expect("permitted");

    assert_eq!(update.facts_written, 0);
    assert_eq!(
        update.unwritten,
        vec!["insufficient_evidence:coding:1/3".to_string()]
    );
}

#[test]
fn confidence_moves_by_at_most_one_drift_step_per_refresh() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let dir = tempfile::tempdir().unwrap();
    archon_cognitive::ensure_cognitive_schema(&db).unwrap();
    // Four verified successes: the target is 1.0, which is 0.5 away from the
    // neutral start. One refresh may move only `MAX_CONFIDENCE_DRIFT`.
    for id in ["r1", "r2", "r3", "r4"] {
        insert_reflection(&db, id, SituationKind::CodeChange, "success");
    }
    let writer = SelfModelWriter::new(&db, dir.path(), Some(policy(true)));

    let update = writer.refresh_domain_trust().unwrap().expect("permitted");

    assert_eq!(update.facts_written, 1);
    assert!(update.changed());
    let confidence = trust_confidence(&db, "coding").expect("fact");
    assert!(
        (confidence - (0.5 + MAX_CONFIDENCE_DRIFT)).abs() < 1e-6,
        "confidence jumped to {confidence}"
    );

    // A second refresh over the same evidence drifts once more, still bounded,
    // and still nowhere near the 1.0 target.
    let second = writer.refresh_domain_trust().unwrap().expect("permitted");
    assert_eq!(second.facts_written, 1);
    let confidence = trust_confidence(&db, "coding").expect("fact");
    assert!((confidence - (0.5 + 2.0 * MAX_CONFIDENCE_DRIFT)).abs() < 1e-6);
}

#[test]
fn a_written_fact_is_readable_through_the_store_and_emits_a_metric() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let dir = tempfile::tempdir().unwrap();
    archon_cognitive::ensure_cognitive_schema(&db).unwrap();
    for id in ["r1", "r2", "r3"] {
        insert_reflection(&db, id, SituationKind::Research, "failure");
    }

    let update = SelfModelWriter::new(&db, dir.path(), Some(policy(true)))
        .refresh_domain_trust()
        .unwrap()
        .expect("permitted");

    assert_eq!(update.facts_written, 1);
    assert_eq!(update.metrics_emitted, 1);
    assert!(update.errors.is_empty(), "{:?}", update.errors);

    // The fact the CLI reads as "Self-model facts" is now non-zero.
    let briefing = SelfModelStore::new(&db).unwrap().export_briefing().unwrap();
    assert_eq!(briefing.fact_count, 1);

    let snapshot = archon_cognitive::MetricEventStore::new(&db, dir.path())
        .unwrap()
        .latest_snapshot()
        .unwrap();
    assert_eq!(
        snapshot
            .pooled("self_model_fact_update_count")
            .unwrap()
            .value,
        Some(1.0)
    );
    let confidence = snapshot
        .pooled("self_model_fact_confidence_mean")
        .unwrap()
        .value
        .unwrap();
    // All failures, so the target is 0.0 and the fact drifts downward.
    assert!(confidence < 0.5, "{confidence}");
}

/// Re-running with no new evidence and no remaining drift must not rewrite the
/// fact: a fresh `last_seen_at` would imply evidence that never arrived.
#[test]
fn an_unchanged_domain_is_not_rewritten() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let dir = tempfile::tempdir().unwrap();
    archon_cognitive::ensure_cognitive_schema(&db).unwrap();
    // Target 0.5 equals the neutral start, so there is no drift to apply after
    // the first write.
    for (id, outcome) in [
        ("r1", "success"),
        ("r2", "failure"),
        ("r3", "success"),
        ("r4", "failure"),
    ] {
        insert_reflection(&db, id, SituationKind::CodeChange, outcome);
    }
    let writer = SelfModelWriter::new(&db, dir.path(), Some(policy(true)));

    let first = writer.refresh_domain_trust().unwrap().expect("permitted");
    let second = writer.refresh_domain_trust().unwrap().expect("permitted");

    // The first write is real: a 50% rate over four verified outcomes is a
    // measurement, and `evidence_count` is what tells it apart from a
    // placeholder at the same confidence.
    assert_eq!(first.facts_written, 1);
    // The second has nothing new to say, so it says nothing.
    assert_eq!(second.facts_written, 0);
    assert_eq!(second.unwritten, vec!["unchanged:coding:4".to_string()]);
    assert!((trust_confidence(&db, "coding").unwrap() - 0.5).abs() < 1e-6);
}
