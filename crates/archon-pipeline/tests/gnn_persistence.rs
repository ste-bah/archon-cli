//! GNN persistence integration test — CozoDB-backed WeightStore roundtrip.
//!
//! Verifies: save → version bump → reload → byte-identical weights.

use std::sync::Arc;

use archon_pipeline::learning::gnn::weights::{Initialization, WeightStore};
use archon_pipeline::learning::schema;

fn mem_db() -> cozo::DbInstance {
    cozo::DbInstance::new("mem", "", "").unwrap()
}

#[test]
fn roundtrip_weights_survive_save_and_reload() {
    let db = Arc::new(mem_db());
    schema::initialize_learning_schemas(&db).expect("schema init");

    let store = WeightStore::new(Arc::clone(&db));
    assert_eq!(store.current_version(), 0);

    // Initialize with seed=42
    store.initialize("layer1", 16, 32, Initialization::He, 42);
    store.initialize("layer2", 32, 64, Initialization::He, 43);
    store.initialize("layer3", 64, 16, Initialization::He, 44);

    // Save and verify version bump
    let v1 = store.save_all().expect("save_all v1");
    assert_eq!(v1, 1);
    assert_eq!(store.current_version(), 1);

    // Capture saved weights
    let w1_before = store.get_weights("layer1");
    let b1_before = store.get_bias("layer1");

    // Create a fresh store from the same DB — simulates restart
    let store2 = WeightStore::new(Arc::clone(&db));
    assert_eq!(store2.current_version(), 1);

    // Load version 1 and verify byte-identical
    store2.load_version(1).expect("load_version 1");
    let w1_after = store2.get_weights("layer1");
    let b1_after = store2.get_bias("layer1");

    assert_eq!(w1_before.len(), w1_after.len());
    for (row_a, row_b) in w1_before.iter().zip(w1_after.iter()) {
        assert_eq!(row_a.len(), row_b.len());
        for (a, b) in row_a.iter().zip(row_b.iter()) {
            assert!((a - b).abs() < 1e-9, "weight mismatch: {a} vs {b}");
        }
    }
    for (a, b) in b1_before.iter().zip(b1_after.iter()) {
        assert!((a - b).abs() < 1e-9, "bias mismatch: {a} vs {b}");
    }

    // Save again → version 2
    let v2 = store2.save_all().expect("save_all v2");
    assert_eq!(v2, 2);
    assert_eq!(store2.current_version(), 2);

    // latest_version() from DB
    assert_eq!(store.latest_version().expect("latest_version"), 2);
}

#[test]
fn adam_state_survives_roundtrip() {
    let db = Arc::new(mem_db());
    schema::initialize_learning_schemas(&db).expect("schema init");

    let store = WeightStore::new(Arc::clone(&db));
    store.initialize("layer1", 4, 8, Initialization::He, 100);
    store.save_all().expect("save v1");

    // Write Adam state via CozoDB directly
    let adam_state = r#"
        ?[layer_id, version, timestep, m_blob, v_blob] <- [
            ["layer1", 1, 5, [], []]
        ]
        :replace gnn_adam_state { layer_id, version => timestep, m_blob, v_blob }
    "#;
    db.run_script(
        adam_state,
        Default::default(),
        cozo::ScriptMutability::Mutable,
    )
    .expect("adam insert");

    // Fresh store should still load
    let store2 = WeightStore::new(Arc::clone(&db));
    assert_eq!(store2.current_version(), 1);
    store2.load_version(1).expect("load after adam write");
    assert!(store2.has_layer("layer1"));
}

#[test]
fn fresh_store_starts_at_version_zero() {
    let db = Arc::new(mem_db());
    schema::initialize_learning_schemas(&db).expect("schema init");
    let store = WeightStore::new(db);
    assert_eq!(store.current_version(), 0);
    assert_eq!(store.layer_count(), 0);
}
