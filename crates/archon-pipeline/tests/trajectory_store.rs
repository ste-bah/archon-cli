use archon_pipeline::learning::schema::initialize_learning_schemas;
use archon_pipeline::learning::sona::Trajectory;
use archon_pipeline::learning::trajectory_store;
use cozo::{DataValue, DbInstance, ScriptMutability};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn mem_db() -> DbInstance {
    DbInstance::new("mem", "", "").unwrap()
}

fn make_trajectory(id: &str, route: &str, quality: f64) -> Trajectory {
    Trajectory {
        trajectory_id: id.to_string(),
        route: route.to_string(),
        agent_key: "test-agent".to_string(),
        session_id: "sess-1".to_string(),
        patterns: vec!["pat-1".to_string()],
        context: vec!["ctx-1".to_string()],
        embedding: vec![1.0_f32, 2.0, 3.0],
        quality,
        reward: 1.2,
        feedback_score: 0.9,
        weights_path: "/w/test.bin".to_string(),
        created_at: 1700000000,
        updated_at: 1700000000,
    }
}

// ---------------------------------------------------------------------------
// Test 1: store_trajectory_persists_all_13_fields
// ---------------------------------------------------------------------------

#[test]
fn store_trajectory_persists_all_13_fields() {
    let db = mem_db();
    initialize_learning_schemas(&db).unwrap();

    let traj = Trajectory {
        trajectory_id: "test-id".to_string(),
        route: "test.route".to_string(),
        agent_key: "test-agent".to_string(),
        session_id: "sess-1".to_string(),
        patterns: vec!["pat-1".to_string()],
        context: vec!["ctx-1".to_string()],
        embedding: vec![1.0_f32, 2.0, 3.0],
        quality: 0.85,
        reward: 1.2,
        feedback_score: 0.9,
        weights_path: "/w/test.bin".to_string(),
        created_at: 1700000000,
        updated_at: 1700000000,
    };

    trajectory_store::store_trajectory(&db, &traj).unwrap();

    let query = r#"
        ?[trajectory_id, route, agent_key, session_id, patterns, context, embedding,
          quality, reward, feedback_score, weights_path, created_at, updated_at] :=
            *trajectories[trajectory_id, route, agent_key, session_id, patterns, context,
              embedding, quality, reward, feedback_score, weights_path, created_at, updated_at]
    "#;

    let result = db
        .run_script(query, Default::default(), ScriptMutability::Immutable)
        .unwrap();
    assert_eq!(result.rows.len(), 1, "should have exactly one row");

    let row = &result.rows[0];

    // Column 0: trajectory_id (String)
    assert_eq!(row[0].get_str().unwrap(), "test-id");
    // Column 1: route (String)
    assert_eq!(row[1].get_str().unwrap(), "test.route");
    // Column 2: agent_key (String)
    assert_eq!(row[2].get_str().unwrap(), "test-agent");
    // Column 3: session_id (String)
    assert_eq!(row[3].get_str().unwrap(), "sess-1");

    // Column 4: patterns ([String])
    match &row[4] {
        DataValue::List(list) => {
            assert_eq!(list.len(), 1, "patterns should have 1 element");
            assert_eq!(list[0].get_str().unwrap(), "pat-1");
        }
        other => panic!("expected DataValue::List for patterns, got {:?}", other),
    }

    // Column 5: context ([String])
    match &row[5] {
        DataValue::List(list) => {
            assert_eq!(list.len(), 1, "context should have 1 element");
            assert_eq!(list[0].get_str().unwrap(), "ctx-1");
        }
        other => panic!("expected DataValue::List for context, got {:?}", other),
    }

    // Column 6: embedding ([Float])
    match &row[6] {
        DataValue::List(list) => {
            assert_eq!(list.len(), 3, "embedding should have 3 elements");
            assert!((list[0].get_float().unwrap() - 1.0).abs() < f64::EPSILON);
            assert!((list[1].get_float().unwrap() - 2.0).abs() < f64::EPSILON);
            assert!((list[2].get_float().unwrap() - 3.0).abs() < f64::EPSILON);
        }
        other => panic!("expected DataValue::List for embedding, got {:?}", other),
    }

    // Column 7: quality (Float)
    assert!((row[7].get_float().unwrap() - 0.85).abs() < f64::EPSILON);
    // Column 8: reward (Float)
    assert!((row[8].get_float().unwrap() - 1.2).abs() < f64::EPSILON);
    // Column 9: feedback_score (Float)
    assert!((row[9].get_float().unwrap() - 0.9).abs() < f64::EPSILON);
    // Column 10: weights_path (String)
    assert_eq!(row[10].get_str().unwrap(), "/w/test.bin");
    // Column 11: created_at (Int)
    assert_eq!(row[11].get_int().unwrap(), 1_700_000_000);
    // Column 12: updated_at (Int)
    assert_eq!(row[12].get_int().unwrap(), 1_700_000_000);
}

// ---------------------------------------------------------------------------
// Test 2: store_trajectory_overwrites_on_same_id
// ---------------------------------------------------------------------------

#[test]
fn store_trajectory_overwrites_on_same_id() {
    let db = mem_db();
    initialize_learning_schemas(&db).unwrap();

    // First store
    let traj1 = make_trajectory("dup-id", "first.route", 0.5);
    trajectory_store::store_trajectory(&db, &traj1).unwrap();

    // Second store with same trajectory_id, different route/quality
    let traj2 = make_trajectory("dup-id", "second.route", 0.9);
    trajectory_store::store_trajectory(&db, &traj2).unwrap();

    // Should have exactly one row (upsert semantics)
    let count_query = r#"
        ?[count(trajectory_id)] := *trajectories[
            trajectory_id, _, _, _, _, _, _, _, _, _, _, _, _]
    "#;
    let result = db
        .run_script(count_query, Default::default(), ScriptMutability::Immutable)
        .unwrap();
    assert_eq!(result.rows[0][0].get_int().unwrap(), 1);

    // Verify the second values won
    let check = r#"
        ?[trajectory_id, route, quality] :=
            *trajectories[trajectory_id, route, _, _, _, _, _, quality, _, _, _, _, _]
    "#;
    let result = db
        .run_script(check, Default::default(), ScriptMutability::Immutable)
        .unwrap();
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0].get_str().unwrap(), "dup-id");
    assert_eq!(result.rows[0][1].get_str().unwrap(), "second.route");
    assert!((result.rows[0][2].get_float().unwrap() - 0.9).abs() < f64::EPSILON);
}

// ---------------------------------------------------------------------------
// Test 3: store_trajectory_batch_persists_n_rows
// ---------------------------------------------------------------------------

#[test]
fn store_trajectory_batch_persists_n_rows() {
    let db = mem_db();
    initialize_learning_schemas(&db).unwrap();

    let trajs = vec![
        make_trajectory("batch-1", "route.a", 0.5),
        make_trajectory("batch-2", "route.b", 0.6),
        make_trajectory("batch-3", "route.c", 0.7),
    ];

    trajectory_store::store_trajectory_batch(&db, &trajs).unwrap();

    // Count rows
    let count_query = r#"
        ?[count(trajectory_id)] := *trajectories[
            trajectory_id, _, _, _, _, _, _, _, _, _, _, _, _]
    "#;
    let result = db
        .run_script(count_query, Default::default(), ScriptMutability::Immutable)
        .unwrap();
    assert_eq!(result.rows[0][0].get_int().unwrap(), 3);

    // Verify each trajectory ID is present
    let ids_query = r#"
        ?[trajectory_id] := *trajectories[
            trajectory_id, _, _, _, _, _, _, _, _, _, _, _, _]
    "#;
    let result = db
        .run_script(ids_query, Default::default(), ScriptMutability::Immutable)
        .unwrap();
    let mut ids: Vec<&str> = result
        .rows
        .iter()
        .map(|row| row[0].get_str().unwrap())
        .collect();
    ids.sort();
    assert_eq!(ids, vec!["batch-1", "batch-2", "batch-3"]);
}

// ---------------------------------------------------------------------------
// Test 4: store_trajectory_with_empty_embedding_succeeds_then_filtered_by_query
// ---------------------------------------------------------------------------

#[test]
fn store_trajectory_with_empty_embedding_succeeds_then_filtered_by_query() {
    let db = mem_db();
    initialize_learning_schemas(&db).unwrap();

    let traj = Trajectory {
        trajectory_id: "empty-emb".to_string(),
        route: "test.route".to_string(),
        agent_key: "test-agent".to_string(),
        session_id: "sess-1".to_string(),
        patterns: vec![],
        context: vec![],
        embedding: vec![],
        quality: 0.5,
        reward: 0.5,
        feedback_score: 0.5,
        weights_path: String::new(),
        created_at: 1700000000,
        updated_at: 1700000000,
    };

    trajectory_store::store_trajectory(&db, &traj).unwrap();

    // Verify it was stored: count should be 1
    let count_all = r#"
        ?[count(trajectory_id)] := *trajectories[
            trajectory_id, _, _, _, _, _, _, _, _, _, _, _, _]
    "#;
    let result = db
        .run_script(count_all, Default::default(), ScriptMutability::Immutable)
        .unwrap();
    assert_eq!(
        result.rows[0][0].get_int().unwrap(),
        1,
        "trajectory with empty embedding should be stored"
    );

    // Filter by non-empty embedding: count should be 0
    let count_nonempty = r#"
        ?[count(trajectory_id)] := *trajectories[
            trajectory_id, _, _, _, _, _, embedding, _, _, _, _, _, _],
            is_list(embedding), length(embedding) > 0
    "#;
    let result = db
        .run_script(
            count_nonempty,
            Default::default(),
            ScriptMutability::Immutable,
        )
        .unwrap();
    assert_eq!(
        result.rows[0][0].get_int().unwrap(),
        0,
        "no trajectories should have non-empty embedding"
    );
}
