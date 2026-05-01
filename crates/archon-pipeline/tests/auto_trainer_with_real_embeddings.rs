//! Integration tests for trajectory embedding persistence and query filtering
//! after the v0.1.36 embedding column migration.

use archon_pipeline::learning::gnn::auto_trainer_runtime::query_trajectories_for_training;
use archon_pipeline::learning::schema::initialize_learning_schemas;
use archon_pipeline::learning::sona::Trajectory;
use archon_pipeline::learning::trajectory_store::store_trajectory;
use cozo::DbInstance;

fn mem_db() -> DbInstance {
    DbInstance::new("mem", "", "").unwrap()
}

fn make_traj(id: &str, embedding: Vec<f32>, quality: f64) -> Trajectory {
    Trajectory {
        trajectory_id: id.to_string(),
        route: "test.route".to_string(),
        agent_key: "test-agent".to_string(),
        session_id: "sess-1".to_string(),
        patterns: vec!["pat-1".to_string()],
        context: vec!["ctx-1".to_string()],
        embedding,
        quality,
        reward: 1.0,
        feedback_score: 0.9,
        weights_path: "/w/test.bin".to_string(),
        created_at: 1700000000,
        updated_at: 1700000000,
    }
}

#[test]
fn query_trajectories_skips_rows_with_empty_embedding() {
    let db = mem_db();
    initialize_learning_schemas(&db).unwrap();

    store_trajectory(&db, &make_traj("t1", vec![1.0, 2.0, 3.0, 4.0], 0.8)).unwrap();
    store_trajectory(&db, &make_traj("t2", vec![], 0.9)).unwrap();

    let result = query_trajectories_for_training(&db, 4).unwrap();
    assert_eq!(result.len(), 1, "should skip empty-embedding row");
    assert_eq!(result[0].trajectory_id, "t1");
    assert!(!result[0].embedding.is_empty());
}

#[test]
fn query_trajectories_skips_rows_with_wrong_dim() {
    let db = mem_db();
    initialize_learning_schemas(&db).unwrap();

    store_trajectory(&db, &make_traj("t1", vec![1.0, 2.0, 3.0, 4.0], 0.8)).unwrap();
    store_trajectory(&db, &make_traj("t2", vec![1.0, 2.0, 3.0], 0.9)).unwrap(); // wrong dim
    store_trajectory(&db, &make_traj("t3", vec![1.0, 2.0, 3.0, 4.0], 0.0)).unwrap(); // quality=0

    let result = query_trajectories_for_training(&db, 4).unwrap();
    assert_eq!(
        result.len(),
        1,
        "should skip wrong-dim and zero-quality rows"
    );
    assert_eq!(result[0].trajectory_id, "t1");
    assert_eq!(result[0].embedding.len(), 4);
}

#[test]
fn query_returns_3_trajectories_with_distinct_embeddings() {
    let db = mem_db();
    initialize_learning_schemas(&db).unwrap();

    store_trajectory(&db, &make_traj("ta", vec![1.0, 0.0, 0.0, 0.0], 0.7)).unwrap();
    store_trajectory(&db, &make_traj("tb", vec![0.0, 1.0, 0.0, 0.0], 0.8)).unwrap();
    store_trajectory(&db, &make_traj("tc", vec![0.0, 0.0, 1.0, 0.0], 0.9)).unwrap();

    let result = query_trajectories_for_training(&db, 4).unwrap();
    assert_eq!(result.len(), 3);
    // Verify distinct embeddings
    let ids: Vec<&str> = result.iter().map(|t| t.trajectory_id.as_str()).collect();
    assert!(ids.contains(&"ta"));
    assert!(ids.contains(&"tb"));
    assert!(ids.contains(&"tc"));
    // Verify non-zero distinct vectors
    for t in &result {
        assert_eq!(t.embedding.len(), 4);
        assert!(
            t.embedding.iter().any(|&f| f > 0.0),
            "embedding should be non-zero"
        );
    }
}
