use std::sync::{Mutex, MutexGuard, OnceLock};

use super::*;

fn persisted_hnsw_test_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|error| error.into_inner())
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
fn same_key_replacement_uses_in_memory_fallback() {
    let _lock = persisted_hnsw_test_lock();
    let temp = tempfile::tempdir().unwrap();
    let store = DocVectorStore::open(temp.path()).unwrap();
    write_test_vectors(&store, "test", &[("chunk-a", &[1.0, 0.0])]);
    store.build_hnsw("test", 2, None).unwrap();
    write_test_vectors(&store, "test", &[("chunk-a", &[0.0, 1.0])]);

    let hits = store
        .search_persisted_first("test", &[0.0, 1.0], 1, 16, None)
        .unwrap();

    assert_eq!(hits[0].chunk_id, "chunk-a");
    assert!(hits[0].distance < 0.01);
}

#[test]
fn old_manifest_without_generation_is_compatible_but_stale() {
    let _lock = persisted_hnsw_test_lock();
    let temp = tempfile::tempdir().unwrap();
    let store = DocVectorStore::open(temp.path()).unwrap();
    write_test_vectors(&store, "test", &[("chunk-a", &[1.0, 0.0])]);
    let manifest = store.build_hnsw("test", 2, None).unwrap();
    let mut old_manifest = serde_json::to_value(manifest).unwrap();
    old_manifest
        .as_object_mut()
        .unwrap()
        .remove("provider_generation");
    std::fs::write(
        store.hnsw_manifest_path("test"),
        serde_json::to_vec(&old_manifest).unwrap(),
    )
    .unwrap();
    let loads_before = persisted_hnsw_load_count();

    let hits = store
        .search_persisted_first("test", &[1.0, 0.0], 1, 16, None)
        .unwrap();

    assert_eq!(hits[0].chunk_id, "chunk-a");
    assert_eq!(persisted_hnsw_load_count(), loads_before);
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
fn missing_identifier_for_persisted_hit_returns_error() {
    let _lock = persisted_hnsw_test_lock();
    let temp = tempfile::tempdir().unwrap();
    let store = DocVectorStore::open(temp.path()).unwrap();
    write_test_vectors(&store, "test", &[("chunk-a", &[1.0, 0.0])]);
    store.build_hnsw("test", 2, None).unwrap();
    store.db.delete(id_key("test", "chunk-a")).unwrap();

    let error = store
        .search_persisted_first("test", &[1.0, 0.0], 1, 16, None)
        .unwrap_err();

    assert!(error.to_string().contains("missing chunk ID"));
}

#[test]
fn stale_manifest_clears_persisted_snapshot_cache() {
    let _lock = persisted_hnsw_test_lock();
    persisted_hnsw::clear();
    let temp = tempfile::tempdir().unwrap();
    let store = DocVectorStore::open(temp.path()).unwrap();
    write_test_vectors(&store, "test", &[("chunk-a", &[1.0, 0.0])]);
    store.build_hnsw("test", 2, None).unwrap();
    store
        .search_persisted_first("test", &[1.0, 0.0], 1, 16, None)
        .unwrap();
    assert!(persisted_hnsw::cache_present());

    write_test_vectors(&store, "test", &[("chunk-b", &[0.0, 1.0])]);
    store
        .search_persisted_first("test", &[0.0, 1.0], 1, 16, None)
        .unwrap();

    assert!(!persisted_hnsw::cache_present());
    assert_eq!(persisted_hnsw::worker_count(), 0);
}

#[test]
fn persisted_search_holds_snapshot_fence_until_result_is_ready() {
    let _lock = persisted_hnsw_test_lock();
    let temp = tempfile::tempdir().unwrap();
    let store = std::sync::Arc::new(DocVectorStore::open(temp.path()).unwrap());
    write_test_vectors(&store, "test", &[("chunk-a", &[1.0, 0.0])]);
    store.build_hnsw("test", 2, None).unwrap();
    let (fence_acquired, release_fence) = test_hooks::install_snapshot_fence_hook();

    let search_store = store.clone();
    let search = std::thread::spawn(move || {
        search_store
            .search_persisted_first("test", &[1.0, 0.0], 1, 16, None)
            .unwrap()
    });
    fence_acquired.recv().unwrap();
    assert!(store.snapshot_fence.try_lock().is_err());
    let writer_store = store.clone();
    let writer = std::thread::spawn(move || {
        write_test_vectors(&writer_store, "test", &[("chunk-a", &[0.0, 1.0])]);
    });
    release_fence.send(()).unwrap();

    assert_eq!(search.join().unwrap()[0].chunk_id, "chunk-a");
    writer.join().unwrap();
    test_hooks::clear_snapshot_fence_hook();
}

#[test]
fn persisted_snapshot_cache_is_shared_across_search_threads() {
    let _lock = persisted_hnsw_test_lock();
    let temp = tempfile::tempdir().unwrap();
    let store = std::sync::Arc::new(DocVectorStore::open(temp.path()).unwrap());
    write_test_vectors(&store, "test", &[("chunk-a", &[1.0, 0.0])]);
    store.build_hnsw("test", 2, None).unwrap();
    let loads_before = persisted_hnsw_load_count();

    let first_store = store.clone();
    let first = std::thread::spawn(move || {
        first_store
            .search_persisted_first("test", &[1.0, 0.0], 1, 16, None)
            .unwrap()
    });
    let second_store = store.clone();
    let second = std::thread::spawn(move || {
        second_store
            .search_persisted_first("test", &[1.0, 0.0], 1, 16, None)
            .unwrap()
    });

    assert_eq!(first.join().unwrap()[0].chunk_id, "chunk-a");
    assert_eq!(second.join().unwrap()[0].chunk_id, "chunk-a");
    assert_eq!(persisted_hnsw_load_count(), loads_before + 1);
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
