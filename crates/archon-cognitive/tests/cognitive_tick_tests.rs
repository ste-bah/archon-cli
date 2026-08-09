use archon_cognitive::{CognitiveTick, SituationKind};
use archon_policy::CognitivePolicy;
use chrono::Utc;
use cozo::{DbInstance, ScriptMutability};

fn policy(allow_tick: bool) -> CognitivePolicy {
    CognitivePolicy {
        enabled: true,
        allow_autonomous_tick: allow_tick,
        allow_autonomous_low_risk_apply: true,
        max_autonomous_risk: "Low".into(),
        ..CognitivePolicy::default()
    }
}

fn insert_reflection(db: &DbInstance, id: &str, lesson: &str, kind: SituationKind) {
    let script = format!(
        "?[reflection_id, session_id, turn_number, decision_id, situation_kind, attempted, worked, failed, outcome, lesson, should_propose, proposed_rule_id, created_at] <- \
         [['{id}', 'session-1', 1, 'decision-{id}', '{}', 'attempted', '', 'failed', 'failure', '{lesson}', true, '', '{}']]
         :put cognitive_reflections {{ reflection_id => session_id, turn_number, decision_id, situation_kind, attempted, worked, failed, outcome, lesson, should_propose, proposed_rule_id, created_at }}",
        kind.as_str(),
        Utc::now().to_rfc3339()
    );
    db.run_script(&script, Default::default(), ScriptMutability::Mutable)
        .unwrap();
}

fn audit_column(db: &DbInstance, column: &str) -> cozo::DataValue {
    db.run_script(
        format!("?[value] := *cognitive_tick_audit{{tick_id, {column}: value}}").as_str(),
        Default::default(),
        ScriptMutability::Immutable,
    )
    .unwrap()
    .rows
    .remove(0)
    .remove(0)
}

fn count(db: &DbInstance, relation: &str, key: &str) -> usize {
    db.run_script(
        format!("?[id] := *{relation}{{{key}: id}}").as_str(),
        Default::default(),
        ScriptMutability::Immutable,
    )
    .unwrap()
    .rows
    .len()
}

#[test]
fn disabled_tick_fails_closed_and_writes_audit() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let tick = CognitiveTick::new(&db, Some(policy(false))).unwrap();

    let report = tick.tick().unwrap();

    assert!(report.errors.contains(&"tick disabled by policy".into()));
    assert_eq!(count(&db, "cognitive_tick_audit", "tick_id"), 1);
}

#[test]
fn enabled_tick_records_compact_audit() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let tick = CognitiveTick::new(&db, Some(policy(true))).unwrap();

    let report = tick.tick().unwrap();

    assert!(report.errors.is_empty());
    assert_eq!(count(&db, "cognitive_tick_audit", "tick_id"), 1);
}

/// The self-model step performs no work, so a tick must not claim it did.
/// `Some(true)` here would be the fabricated success that made this field
/// carry no information on every tick ever recorded.
#[test]
fn tick_does_not_claim_the_self_model_was_updated() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let tick = CognitiveTick::new(&db, Some(policy(true))).unwrap();

    let report = tick.tick().unwrap();

    assert_eq!(report.self_model_updated, None);
    assert_ne!(report.self_model_updated, Some(true));
    assert_eq!(
        audit_column(&db, "self_model_updated"),
        cozo::DataValue::Null
    );
    // Nothing wrote a self-model fact, which is why there is nothing to report.
    assert_eq!(count(&db, "self_model_facts", "fact_id"), 0);
}

/// Dead-letter replay is unimplemented, so its result must stay distinguishable
/// from a tick that really did inspect an empty queue and measured zero.
#[test]
fn tick_reports_dead_letter_replay_as_unmeasured_not_zero() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let tick = CognitiveTick::new(&db, Some(policy(true))).unwrap();

    let report = tick.tick().unwrap();

    assert_eq!(report.dead_letters_replayed, None);
    assert_ne!(report.dead_letters_replayed, Some(0));
    assert_eq!(
        audit_column(&db, "dead_letters_replayed"),
        cozo::DataValue::Null
    );
}

/// The JSON surface consumers read must carry the same distinction; a serialised
/// `0`/`false` would put the fabrication straight back.
#[test]
fn tick_report_json_marks_unmeasured_steps_as_null() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let tick = CognitiveTick::new(&db, Some(policy(true))).unwrap();

    let report = tick.tick().unwrap();
    let json: serde_json::Value =
        serde_json::from_str(&serde_json::to_string(&report).unwrap()).unwrap();

    assert!(json["self_model_updated"].is_null());
    assert!(json["dead_letters_replayed"].is_null());
}

#[test]
fn tick_generates_one_proposal_per_repeated_lesson() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let tick = CognitiveTick::new(&db, Some(policy(true))).unwrap();
    for id in ["r1", "r2", "r3"] {
        insert_reflection(
            &db,
            id,
            "answer format should include compact evidence",
            SituationKind::Greeting,
        );
    }

    let report = tick.tick().unwrap();

    assert_eq!(report.proposals_generated, 1);
    assert_eq!(count(&db, "governed_proposals", "proposal_id"), 1);
}
