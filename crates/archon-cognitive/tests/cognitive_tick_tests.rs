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
    assert!(report.self_model_updated);
    assert_eq!(count(&db, "cognitive_tick_audit", "tick_id"), 1);
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
