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
