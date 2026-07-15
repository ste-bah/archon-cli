use std::sync::{Mutex, MutexGuard, OnceLock};

use super::*;

fn persisted_hnsw_test_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
}

fn write_test_vectors(store: &DocVectorStore, provider: &str, rows: &[(&str, &[f32])]) {
    let writes: Vec<_> = rows
        .iter()
        .map(|(chunk_id, embedding)| VectorWrite {
            chunk_id,
            content_hash: chunk_id,
            provider,
            embedding,
        })
        .collect();
    store.put_vectors(&writes).unwrap();
}

#[test]
fn persisted_snapshot_returns_nearest_chunk() {
    let _lock = persisted_hnsw_test_lock();
    let temp = tempfile::tempdir().unwrap();
    let store = DocVectorStore::open(temp.path()).unwrap();
    write_test_vectors(
        &store,
        "test",
        &[("chunk-a", &[1.0, 0.0]), ("chunk-b", &[0.0, 1.0])],
    );
    store.build_hnsw("test", 2, None).unwrap();

    let hits = store
        .search_persisted_first("test", &[0.99, 0.01], 1, 16, None)
        .unwrap();

    assert_eq!(hits[0].chunk_id, "chunk-a");
}

#[test]
fn missing_manifest_uses_in_memory_fallback() {
    let _lock = persisted_hnsw_test_lock();
    let temp = tempfile::tempdir().unwrap();
    let store = DocVectorStore::open(temp.path()).unwrap();
    write_test_vectors(&store, "test", &[("chunk-a", &[1.0, 0.0])]);

    let hits = store
        .search_persisted_first("test", &[1.0, 0.0], 1, 16, None)
        .unwrap();

    assert_eq!(hits[0].chunk_id, "chunk-a");
}

#[test]
fn stale_vector_count_uses_in_memory_fallback_and_sees_new_data() {
    let _lock = persisted_hnsw_test_lock();
    let temp = tempfile::tempdir().unwrap();
    let store = DocVectorStore::open(temp.path()).unwrap();
    write_test_vectors(&store, "test", &[("chunk-a", &[1.0, 0.0])]);
    store.build_hnsw("test", 2, None).unwrap();
    write_test_vectors(&store, "test", &[("chunk-b", &[0.0, 1.0])]);

    let hits = store
        .search_persisted_first("test", &[0.0, 1.0], 1, 16, None)
        .unwrap();

    assert_eq!(hits[0].chunk_id, "chunk-b");
}

#[test]
fn corrupt_current_dump_returns_error() {
    let _lock = persisted_hnsw_test_lock();
    let temp = tempfile::tempdir().unwrap();
    let store = DocVectorStore::open(temp.path()).unwrap();
    write_test_vectors(&store, "test", &[("chunk-a", &[1.0, 0.0])]);
    let manifest = store.build_hnsw("test", 2, None).unwrap();
    std::fs::write(
        store
            .hnsw_dir("test")
            .join(format!("{}.hnsw.graph", manifest.dump_basename)),
        b"corrupt",
    )
    .unwrap();

    let error = store
        .search_persisted_first("test", &[1.0, 0.0], 1, 16, None)
        .unwrap_err();

    assert!(error.to_string().contains("load persisted HNSW"));
}

#[test]
fn provider_or_dimension_mismatch_uses_in_memory_fallback() {
    let _lock = persisted_hnsw_test_lock();
    let temp = tempfile::tempdir().unwrap();
    let store = DocVectorStore::open(temp.path()).unwrap();
    write_test_vectors(&store, "test", &[("chunk-a", &[1.0, 0.0])]);
    store.build_hnsw("test", 2, None).unwrap();
    write_test_vectors(&store, "other", &[("other-a", &[0.0, 1.0])]);

    let provider_hits = store
        .search_persisted_first("other", &[0.0, 1.0], 1, 16, None)
        .unwrap();
    let dimension_hits = store
        .search_persisted_first("test", &[1.0, 0.0, 0.0], 1, 16, None)
        .unwrap_err();

    assert_eq!(provider_hits[0].chunk_id, "other-a");
    assert!(
        dimension_hits
            .to_string()
            .contains("vector dimension mismatch")
    );
}

#[test]
fn repeated_persisted_search_reuses_loaded_snapshot() {
    let _lock = persisted_hnsw_test_lock();
    let temp = tempfile::tempdir().unwrap();
    let store = DocVectorStore::open(temp.path()).unwrap();
    write_test_vectors(&store, "test", &[("chunk-a", &[1.0, 0.0])]);
    store.build_hnsw("test", 2, None).unwrap();

    let loads_before = persisted_hnsw_load_count();
    store
        .search_persisted_first("test", &[1.0, 0.0], 1, 16, None)
        .unwrap();
    store
        .search_persisted_first("test", &[1.0, 0.0], 1, 16, None)
        .unwrap();

    assert_eq!(persisted_hnsw_load_count(), loads_before + 1);
}
