//! Live smoke test for trajectory embeddings — TASK-GHOST-003 Gate 5.
//! Exercises store → query → filter end-to-end with a real CozoDB instance.

use archon_pipeline::learning::gnn::auto_trainer_runtime::query_trajectories_for_training;
use archon_pipeline::learning::schema::initialize_learning_schemas;
use archon_pipeline::learning::sona::Trajectory;
use archon_pipeline::learning::trajectory_store::store_trajectory;
use cozo::DbInstance;

fn main() {
    let db = DbInstance::new("mem", "", "").expect("create in-memory CozoDB");

    // Initialize schemas (includes migration to v2)
    initialize_learning_schemas(&db).expect("initialize learning schemas");

    // Store trajectories with embeddings
    let t1 = Trajectory {
        trajectory_id: "smoke-t1".into(),
        route: "test.smoke".into(),
        agent_key: "smoke-agent".into(),
        session_id: "smoke-sess".into(),
        patterns: vec!["pat-a".into()],
        context: vec!["ctx-a".into()],
        embedding: vec![1.0, 2.0, 3.0, 4.0],
        quality: 0.85,
        reward: 1.0,
        feedback_score: 0.9,
        weights_path: "/tmp/smoke.bin".into(),
        created_at: 1700000000,
        updated_at: 1700000000,
    };
    store_trajectory(&db, &t1).expect("store t1");

    let t2 = Trajectory {
        trajectory_id: "smoke-t2".into(),
        route: "test.smoke".into(),
        agent_key: "smoke-agent".into(),
        session_id: "smoke-sess".into(),
        patterns: vec!["pat-b".into()],
        context: vec!["ctx-b".into()],
        embedding: vec![],
        quality: 0.9,
        reward: 1.0,
        feedback_score: 0.95,
        weights_path: "/tmp/smoke2.bin".into(),
        created_at: 1700000000,
        updated_at: 1700000000,
    };
    store_trajectory(&db, &t2).expect("store t2");

    let t3 = Trajectory {
        trajectory_id: "smoke-t3".into(),
        route: "test.smoke".into(),
        agent_key: "smoke-agent".into(),
        session_id: "smoke-sess".into(),
        patterns: vec!["pat-c".into()],
        context: vec!["ctx-c".into()],
        embedding: vec![5.0, 6.0, 7.0, 8.0],
        quality: 0.95,
        reward: 1.0,
        feedback_score: 0.98,
        weights_path: "/tmp/smoke3.bin".into(),
        created_at: 1700000000,
        updated_at: 1700000000,
    };
    store_trajectory(&db, &t3).expect("store t3");

    // Query for training — should skip t2 (empty embedding)
    let results = query_trajectories_for_training(&db, 4).expect("query trajectories");
    println!(
        "Queried {} trajectories for training (expected 2)",
        results.len()
    );
    assert_eq!(results.len(), 2, "should skip empty-embedding row");
    assert!(results.iter().any(|t| t.trajectory_id == "smoke-t1"));
    assert!(results.iter().any(|t| t.trajectory_id == "smoke-t3"));

    // Verify embeddings survived round-trip
    for t in &results {
        println!(
            "  {} embedding={:?} quality={}",
            t.trajectory_id, t.embedding, t.quality
        );
        assert_eq!(t.embedding.len(), 4);
        assert!(t.embedding.iter().any(|&f| f > 0.0));
    }

    // Verify migration version is 2
    let version = db
        .run_script(
            "?[v] := *learning_schema_version{component: \"trajectories\", version: v}",
            Default::default(),
            cozo::ScriptMutability::Immutable,
        )
        .expect("read version");
    let v = version.rows[0][0].get_int().unwrap();
    println!("Schema version: {v}");
    assert_eq!(v, 2, "schema version should be 2");

    // Verify idempotent — second init is no-op
    initialize_learning_schemas(&db).expect("second init");

    println!(
        "\nSMOKE TEST PASSED — Trajectory embeddings: store, query, filter, migration verified end-to-end."
    );
}
