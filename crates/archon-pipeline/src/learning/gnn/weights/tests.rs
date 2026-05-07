use super::serialization::{
    deserialize_matrix, deserialize_vector, serialize_matrix, serialize_vector,
};
use super::*;

// ---- WeightStore (in-memory) ----

#[test]
fn test_weight_store_initialize_and_get() {
    let store = WeightStore::with_in_memory();
    store.initialize("test_layer", 4, 2, Initialization::He, 42);
    let w = store.get_weights("test_layer");
    assert_eq!(w.len(), 2);
    assert_eq!(w[0].len(), 4);
    assert!(w.iter().any(|row| row.iter().any(|&x| x != 0.0)));
}

#[test]
fn test_weight_store_different_seeds() {
    let s1 = WeightStore::with_in_memory();
    s1.initialize("l", 4, 2, Initialization::He, 1);
    let w1 = s1.get_weights("l");

    let s2 = WeightStore::with_in_memory();
    s2.initialize("l", 4, 2, Initialization::He, 2);
    let w2 = s2.get_weights("l");

    let diff = w1
        .iter()
        .flatten()
        .zip(w2.iter().flatten())
        .any(|(a, b)| (a - b).abs() > 1e-6);
    assert!(diff);
}

#[test]
fn test_weight_store_set_and_get() {
    let store = WeightStore::with_in_memory();
    let w = vec![vec![1.0, 2.0], vec![3.0, 4.0]];
    let bias = vec![0.1, 0.2];
    store.set_weights("l1", w.clone(), bias.clone());
    assert_eq!(*store.get_weights("l1"), w);
    assert_eq!(*store.get_bias("l1"), bias);
}

#[test]
fn test_weight_store_has_layer() {
    let store = WeightStore::with_in_memory();
    assert!(!store.has_layer("l1"));
    store.initialize("l1", 2, 2, Initialization::Xavier, 0);
    assert!(store.has_layer("l1"));
}

// ---- Serialization round-trip ----

#[test]
fn test_serialize_matrix_roundtrip() {
    let original = vec![
        vec![1.0_f32, -2.5, std::f32::consts::PI],
        vec![0.0, f32::INFINITY, f32::NEG_INFINITY],
    ];
    let bytes = serialize_matrix(&original);
    let restored = deserialize_matrix(&bytes).expect("deserialize failed");
    assert_eq!(restored.len(), 2);
    assert_eq!(restored[0].len(), 3);
    for i in 0..2 {
        for j in 0..3 {
            assert_eq!(original[i][j], restored[i][j], "mismatch at [{},{}]", i, j);
        }
    }
}

#[test]
fn test_serialize_vector_roundtrip() {
    let original = vec![0.0_f32, -1.5, std::f32::consts::PI];
    let bytes = serialize_vector(&original);
    let restored = deserialize_vector(&bytes).expect("deserialize failed");
    assert_eq!(original, restored);
}

#[test]
fn test_deserialize_matrix_empty_input() {
    assert!(deserialize_matrix(&[]).is_none());
    assert!(deserialize_matrix(&[0; 4]).is_none());
}

#[test]
fn test_deserialize_vector_empty_input() {
    assert!(deserialize_vector(&[]).is_none());
}

#[test]
fn test_serialize_empty_matrix() {
    let empty: Vec<Vec<f32>> = vec![];
    let bytes = serialize_matrix(&empty);
    let restored = deserialize_matrix(&bytes).expect("deserialize failed");
    assert!(restored.is_empty());
}

// ---- WeightStoreError ----

#[test]
fn test_weight_store_error_display() {
    assert!(format!("{}", WeightStoreError::NoVersions).contains("No weight versions"));
    assert!(format!("{}", WeightStoreError::VersionNotFound(3)).contains("3"));
    assert!(format!("{}", WeightStoreError::Db("boom".into())).contains("boom"));
    assert!(format!("{}", WeightStoreError::Corrupted("bad".into())).contains("bad"));
    assert!(format!("{}", WeightStoreError::NanWeights("nan".into())).contains("NaN"));
}

#[test]
fn test_in_memory_store_save_all_returns_err() {
    let store = WeightStore::with_in_memory();
    store.initialize("l1", 2, 2, Initialization::He, 42);
    let result = store.save_all();
    assert!(result.is_err());
}

#[test]
fn test_in_memory_store_latest_version_returns_err() {
    let store = WeightStore::with_in_memory();
    let result = store.latest_version();
    assert!(result.is_err());
}

// ---- Legacy WeightManager ----

#[test]
fn test_weight_manager_roundtrip() {
    let original = vec![1.0f32, -2.5, std::f32::consts::PI, 0.0, f32::MIN_POSITIVE];
    let dir = std::env::temp_dir();
    let path = dir.join("test_gnn_weights_pr1.bin");

    WeightManager::save(&original, &path).expect("save failed");
    let loaded = WeightManager::load(&path).expect("load failed");
    assert_eq!(original, loaded);

    let _ = std::fs::remove_file(&path);
}
