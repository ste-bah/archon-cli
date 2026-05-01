//! Integration tests for migration v1 → v2 (add embedding column to trajectories).

use cozo::{DataValue, DbInstance, ScriptMutability};

fn mem_db() -> DbInstance {
    DbInstance::new("mem", "", "").unwrap()
}

fn get_trajectories_version(db: &DbInstance) -> Option<i64> {
    let result = db
        .run_script(
            "?[v] := *learning_schema_version{component: \"trajectories\", version: v}",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .unwrap();
    if result.rows.is_empty() {
        return None;
    }
    Some(result.rows[0][0].get_int().unwrap())
}

fn count_trajectories(db: &DbInstance) -> i64 {
    let result = db
        .run_script(
            "?[count(trajectory_id)] := *trajectories[trajectory_id, _, _, _, _, _, _, _, _, _, _, _, _]",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .unwrap();
    result.rows[0][0].get_int().unwrap()
}

#[test]
fn migration_on_fresh_db_records_version_2() {
    let db = mem_db();
    archon_pipeline::learning::schema::initialize_learning_schemas(&db).unwrap();
    let v = get_trajectories_version(&db);
    assert_eq!(v, Some(2), "fresh DB should record version 2");
}

#[test]
fn migration_on_v1_db_with_3_rows_copies_to_v2_with_empty_embeddings() {
    let db = mem_db();

    // Create v1 trajectories relation (12 cols, no embedding)
    db.run_script(
        ":create trajectories { trajectory_id: String => \
         route: String, agent_key: String, session_id: String, \
         patterns: [String], context: [String], quality: Float, \
         reward: Float, feedback_score: Float, weights_path: String, \
         created_at: Int, updated_at: Int }",
        Default::default(),
        ScriptMutability::Mutable,
    )
    .unwrap();

    // Create version sentinel table and insert v1 sentinel
    db.run_script(
        ":create learning_schema_version { component: String => version: Int, applied_at: Int }",
        Default::default(),
        ScriptMutability::Mutable,
    )
    .unwrap();

    let mut params = std::collections::BTreeMap::new();
    params.insert(
        "component".to_string(),
        DataValue::Str("trajectories".into()),
    );
    params.insert("version".to_string(), DataValue::from(1_i64));
    params.insert("applied_at".to_string(), DataValue::from(1700000000_i64));
    db.run_script(
        "?[component, version, applied_at] <- [[$component, $version, $applied_at]] \
         :put learning_schema_version { component => version, applied_at }",
        params,
        ScriptMutability::Mutable,
    )
    .unwrap();

    // Insert 3 rows into v1 trajectories
    for i in 1..=3 {
        let id = format!("traj-v1-{i}");
        let mut p = std::collections::BTreeMap::new();
        p.insert("trajectory_id".to_string(), DataValue::Str(id.into()));
        p.insert("route".to_string(), DataValue::Str("test".into()));
        p.insert("agent_key".to_string(), DataValue::Str("agent".into()));
        p.insert("session_id".to_string(), DataValue::Str("sess".into()));
        p.insert("patterns".to_string(), DataValue::List(vec![]));
        p.insert("context".to_string(), DataValue::List(vec![]));
        p.insert("quality".to_string(), DataValue::from(0.5_f64));
        p.insert("reward".to_string(), DataValue::from(1.0_f64));
        p.insert("feedback_score".to_string(), DataValue::from(0.8_f64));
        p.insert("weights_path".to_string(), DataValue::Str("".into()));
        p.insert("created_at".to_string(), DataValue::from(1700000000_i64));
        p.insert("updated_at".to_string(), DataValue::from(1700000000_i64));
        db.run_script(
            "?[trajectory_id, route, agent_key, session_id, patterns, context, \
             quality, reward, feedback_score, weights_path, created_at, updated_at] <- \
             [[$trajectory_id, $route, $agent_key, $session_id, $patterns, $context, \
             $quality, $reward, $feedback_score, $weights_path, $created_at, $updated_at]] \
             :put trajectories { trajectory_id => route, agent_key, session_id, \
             patterns, context, quality, reward, feedback_score, weights_path, \
             created_at, updated_at }",
            p,
            ScriptMutability::Mutable,
        )
        .unwrap();
    }

    // Run migration (initialize_learning_schemas triggers apply_pending_migrations)
    archon_pipeline::learning::schema::initialize_learning_schemas(&db).unwrap();

    // Verify 3 rows survived
    assert_eq!(count_trajectories(&db), 3);

    // Verify embeddings are empty (migration backfill)
    let result = db
        .run_script(
            "?[trajectory_id, embedding] := \
             *trajectories[trajectory_id, _, _, _, _, _, embedding, _, _, _, _, _, _]",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .unwrap();
    assert_eq!(result.rows.len(), 3);
    for row in &result.rows {
        match &row[1] {
            DataValue::List(list) => {
                assert!(list.is_empty(), "embedding should be empty after migration")
            }
            _ => panic!("expected list"),
        }
    }

    // Verify version sentinel updated to 2
    assert_eq!(get_trajectories_version(&db), Some(2));
}

#[test]
fn migration_idempotent_running_twice_does_not_fail() {
    let db = mem_db();
    archon_pipeline::learning::schema::initialize_learning_schemas(&db).unwrap();
    archon_pipeline::learning::schema::initialize_learning_schemas(&db).unwrap();
    assert_eq!(get_trajectories_version(&db), Some(2));
}

#[test]
fn migration_on_v2_db_no_op() {
    let db = mem_db();
    archon_pipeline::learning::schema::initialize_learning_schemas(&db).unwrap();
    assert_eq!(get_trajectories_version(&db), Some(2));

    // Should be no-op on second call
    archon_pipeline::learning::schema::initialize_learning_schemas(&db).unwrap();
    assert_eq!(get_trajectories_version(&db), Some(2));
}

#[test]
fn migration_refuses_v3_with_clear_error() {
    let db = mem_db();

    // Create just the version table with v3 sentinel
    db.run_script(
        ":create learning_schema_version { component: String => version: Int, applied_at: Int }",
        Default::default(),
        ScriptMutability::Mutable,
    )
    .unwrap();

    let mut params = std::collections::BTreeMap::new();
    params.insert(
        "component".to_string(),
        DataValue::Str("trajectories".into()),
    );
    params.insert("version".to_string(), DataValue::from(3_i64));
    params.insert("applied_at".to_string(), DataValue::from(1700000000_i64));
    db.run_script(
        "?[component, version, applied_at] <- [[$component, $version, $applied_at]] \
         :put learning_schema_version { component => version, applied_at }",
        params,
        ScriptMutability::Mutable,
    )
    .unwrap();

    // Also need trajectories relation to exist (otherwise :create handles it)
    db.run_script(
        ":create trajectories { trajectory_id: String => \
         route: String, agent_key: String, session_id: String, \
         patterns: [String], context: [String], embedding: [Float], \
         quality: Float, reward: Float, feedback_score: Float, \
         weights_path: String, created_at: Int, updated_at: Int }",
        Default::default(),
        ScriptMutability::Mutable,
    )
    .unwrap();

    let result = archon_pipeline::learning::schema::initialize_learning_schemas(&db);
    assert!(result.is_err(), "should refuse v3 database");
    let msg = format!("{}", result.unwrap_err());
    assert!(
        msg.contains("newer") || msg.contains("upgrade"),
        "error should mention newer/upgrade: {msg}"
    );
}
