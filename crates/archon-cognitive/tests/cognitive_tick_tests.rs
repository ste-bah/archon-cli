use archon_cognitive::{CognitiveTick, SituationKind};
use archon_policy::CognitivePolicy;
use chrono::Utc;
use cozo::{DbInstance, ScriptMutability};

fn policy(allow_tick: bool) -> CognitivePolicy {
    CognitivePolicy {
        enabled: true,
        allow_autonomous_tick: allow_tick,
        allow_autonomous_low_risk_apply: true,
        allow_self_model_updates: true,
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
    let dir = tempfile::tempdir().unwrap();
    let tick = CognitiveTick::new(&db, Some(policy(false)), dir.path()).unwrap();

    let report = tick.tick().unwrap();

    assert!(report.errors.contains(&"tick disabled by policy".into()));
    assert_eq!(count(&db, "cognitive_tick_audit", "tick_id"), 1);
}

#[test]
fn enabled_tick_records_compact_audit() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let tick = CognitiveTick::new(&db, Some(policy(true)), dir.path()).unwrap();

    let report = tick.tick().unwrap();

    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(count(&db, "cognitive_tick_audit", "tick_id"), 1);
}

/// The self-model step now runs, so it must report what it found: `false` for
/// "it ran and nothing had enough evidence to change", never `true` and never
/// a placeholder fact.
#[test]
fn tick_reports_the_self_model_ran_and_changed_nothing() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let tick = CognitiveTick::new(&db, Some(policy(true)), dir.path()).unwrap();

    let report = tick.tick().unwrap();

    assert_eq!(report.self_model_updated, Some(false));
    assert_eq!(
        audit_column(&db, "self_model_updated"),
        cozo::DataValue::Bool(false)
    );
    // No evidence means no fact, not a placeholder fact at neutral confidence.
    assert_eq!(count(&db, "self_model_facts", "fact_id"), 0);
}

/// Policy is the only remaining reason the self-model step reports nothing, and
/// it must stay distinguishable from "ran and found nothing".
#[test]
fn tick_reports_self_model_as_unmeasured_when_policy_withholds_updates() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut denied = policy(true);
    denied.allow_self_model_updates = false;
    let tick = CognitiveTick::new(&db, Some(denied), dir.path()).unwrap();

    let report = tick.tick().unwrap();

    assert_eq!(report.self_model_updated, None);
    assert!(
        report
            .errors
            .contains(&"self_model_updates_not_permitted_by_policy".into())
    );
}

/// `Some(0)` is now a measurement: the ledger and the relation agree.
#[test]
fn tick_measures_an_empty_dead_letter_queue_as_zero() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let tick = CognitiveTick::new(&db, Some(policy(true)), dir.path()).unwrap();

    let report = tick.tick().unwrap();

    assert_eq!(report.dead_letters_replayed, Some(0));
    assert_eq!(
        audit_column(&db, "dead_letters_replayed"),
        cozo::DataValue::from(0)
    );
}

/// A reflection whose Cozo write failed still reaches the ledger. That is the
/// dead-letter queue, and the tick must actually drain it.
#[test]
fn tick_replays_ledgered_reflections_missing_from_the_relation() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let dir = tempfile::tempdir().unwrap();
    archon_cognitive::ensure_cognitive_schema(&db).unwrap();
    let orphan = serde_json::json!({
        "reflection_id": "orphan-1",
        "session_id": "session-1",
        "turn_number": 4,
        "decision_id": "decision-1",
        "situation_kind": "code_change",
        "attempted": "goal:code_change:run_tests",
        "worked": "",
        "failed": "mismatch:repeated_tool_failure:observed_inspect_files",
        "lesson": "code_change: repeated tool failure should stop retrying",
        "should_propose": false,
        "proposed_rule_id": serde_json::Value::Null,
        "outcome": "failure",
        "created_at": Utc::now().to_rfc3339(),
    });
    // The same record twice: the ledger is append-only, so a retried write
    // leaves a duplicate that must still count once.
    std::fs::write(
        dir.path().join("cognitive-reflections.jsonl"),
        format!("{orphan}\n{orphan}\n"),
    )
    .unwrap();
    let tick = CognitiveTick::new(&db, Some(policy(true)), dir.path()).unwrap();

    let report = tick.tick().unwrap();

    assert_eq!(report.dead_letters_replayed, Some(1));
    assert_eq!(count(&db, "cognitive_reflections", "reflection_id"), 1);

    // A second tick finds the relation already caught up.
    let again = CognitiveTick::new(&db, Some(policy(true)), dir.path())
        .unwrap()
        .tick()
        .unwrap();
    assert_eq!(again.dead_letters_replayed, Some(0));
}

#[test]
fn tick_generates_one_proposal_per_repeated_lesson() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let tick = CognitiveTick::new(&db, Some(policy(true)), dir.path()).unwrap();
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
