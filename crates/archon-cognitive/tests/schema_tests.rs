use archon_cognitive::{CURRENT_SCHEMA_VERSION, cognitive_schema_version, ensure_cognitive_schema};
use cozo::{DbInstance, ScriptMutability};

const RELATIONS: &[(&str, &str)] = &[
    ("cognitive_situations", "situation_id"),
    ("cognitive_action_candidates", "candidate_id"),
    ("cognitive_decisions", "decision_id"),
    ("self_model_facts", "fact_id"),
    ("cognitive_reflections", "reflection_id"),
    ("cognitive_prediction_links", "link_id"),
    ("cognitive_policy_state", "state_id"),
    ("cognitive_tick_audit", "tick_id"),
    ("cognitive_metric_events", "metric_event_id"),
    ("cognitive_evaluation_windows", "evaluation_window_id"),
    ("cognitive_causal_lessons", "lesson_id"),
];

#[test]
fn schema_creates_all_cognitive_relations() {
    let db = DbInstance::new("mem", "", "").expect("mem db");
    ensure_cognitive_schema(&db).expect("schema");

    for (relation, key) in RELATIONS {
        let query = format!("?[{key}] := *{relation}{{{key}}}");
        let result = db.run_script(
            query.as_str(),
            Default::default(),
            ScriptMutability::Immutable,
        );
        assert!(result.is_ok(), "relation {relation} should query");
    }
}

#[test]
fn schema_is_idempotent_and_tracks_version() {
    let db = DbInstance::new("mem", "", "").expect("mem db");
    ensure_cognitive_schema(&db).expect("schema 1");
    ensure_cognitive_schema(&db).expect("schema 2");
    ensure_cognitive_schema(&db).expect("schema 3");

    assert_eq!(
        cognitive_schema_version(&db).expect("schema version"),
        CURRENT_SCHEMA_VERSION
    );
}

/// A database created before the tick audit gained nullable measurement columns
/// must be widened in place: `:create` is skipped for an existing relation, so
/// without the migration every later tick write would be rejected.
#[test]
fn schema_migrates_legacy_tick_audit_to_nullable_measurements() {
    let db = DbInstance::new("mem", "", "").expect("mem db");
    db.run_script(
        ":create cognitive_tick_audit { tick_id: String => dead_letters_replayed: Int, \
         proposals_evaluated: Int, proposals_auto_applied: Int, proposals_denied: Int, \
         self_model_updated: Bool, errors_json: String, duration_ms: Int, created_at: String }",
        Default::default(),
        ScriptMutability::Mutable,
    )
    .expect("legacy tick audit relation");
    db.run_script(
        "?[tick_id, dead_letters_replayed, proposals_evaluated, proposals_auto_applied, \
         proposals_denied, self_model_updated, errors_json, duration_ms, created_at] <- \
         [['old-tick', 0, 4, 0, 0, true, '[]', 12, '2026-01-01T00:00:00Z']] \
         :put cognitive_tick_audit { tick_id => dead_letters_replayed, proposals_evaluated, \
         proposals_auto_applied, proposals_denied, self_model_updated, errors_json, duration_ms, \
         created_at }",
        Default::default(),
        ScriptMutability::Mutable,
    )
    .expect("legacy tick row");

    ensure_cognitive_schema(&db).expect("schema migration");

    let rows = db
        .run_script(
            "?[tick_id, dead_letters_replayed, proposals_evaluated, self_model_updated] := \
             *cognitive_tick_audit{tick_id, dead_letters_replayed, proposals_evaluated, \
             self_model_updated}",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .expect("migrated rows")
        .rows;
    assert_eq!(rows.len(), 1, "migration must preserve the audit history");
    assert_eq!(rows[0][0].get_str(), Some("old-tick"));
    // Genuinely measured columns survive untouched.
    assert_eq!(rows[0][2].get_int(), Some(4));
    // The two fabricated columns become "not measured" rather than keeping the
    // hardcoded 0/true that no tick ever observed.
    assert_eq!(rows[0][1], cozo::DataValue::Null);
    assert_eq!(rows[0][3], cozo::DataValue::Null);

    // The whole point of widening: a null write must now be accepted.
    db.run_script(
        "?[tick_id, dead_letters_replayed, proposals_evaluated, proposals_auto_applied, \
         proposals_denied, self_model_updated, errors_json, duration_ms, created_at] <- \
         [['new-tick', null, 1, 0, 0, null, '[]', 3, '2026-02-01T00:00:00Z']] \
         :put cognitive_tick_audit { tick_id => dead_letters_replayed, proposals_evaluated, \
         proposals_auto_applied, proposals_denied, self_model_updated, errors_json, duration_ms, \
         created_at }",
        Default::default(),
        ScriptMutability::Mutable,
    )
    .expect("unmeasured tick write must be accepted after migration");

    // Re-running the schema must not migrate a second time or lose rows.
    ensure_cognitive_schema(&db).expect("schema migration is idempotent");
    assert_eq!(
        db.run_script(
            "?[tick_id] := *cognitive_tick_audit{tick_id}",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .expect("rows after re-run")
        .rows
        .len(),
        2
    );
}

#[test]
fn schema_repairs_legacy_version_relation_shape() {
    let db = DbInstance::new("mem", "", "").expect("mem db");
    db.run_script(
        ":create cognitive_schema_version { version: Int }",
        Default::default(),
        ScriptMutability::Mutable,
    )
    .expect("legacy schema relation");

    ensure_cognitive_schema(&db).expect("schema repair");

    assert_eq!(
        cognitive_schema_version(&db).expect("schema version"),
        CURRENT_SCHEMA_VERSION
    );
}
