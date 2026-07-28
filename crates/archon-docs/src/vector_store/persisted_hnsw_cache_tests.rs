use super::*;

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
fn repeated_persisted_search_scans_reverse_ids_once() {
    let _lock = persisted_hnsw_test_lock();
    let temp = tempfile::tempdir().unwrap();
    let store = DocVectorStore::open(temp.path()).unwrap();
    write_test_vectors(&store, "test", &[("chunk-a", &[1.0, 0.0])]);
    store.build_hnsw("test", 2, None).unwrap();
    reset_reverse_scan_probes();

    store
        .search_persisted_first("test", &[1.0, 0.0], 1, 16, None)
        .unwrap();
    store
        .search_persisted_first("test", &[1.0, 0.0], 1, 16, None)
        .unwrap();

    assert_eq!(reverse_scan_probes(), 1);
}

#[test]
fn replacing_snapshot_releases_previous_worker() {
    let _lock = persisted_hnsw_test_lock();
    let temp = tempfile::tempdir().unwrap();
    let store = DocVectorStore::open(temp.path()).unwrap();
    write_test_vectors(&store, "test", &[("chunk-a", &[1.0, 0.0])]);
    store.build_hnsw("test", 2, None).unwrap();
    store
        .search_persisted_first("test", &[1.0, 0.0], 1, 16, None)
        .unwrap();
    assert_eq!(persisted_hnsw::worker_count(), 1);

    store.build_hnsw("test", 2, None).unwrap();
    wait_for_worker_count(0);
    store
        .search_persisted_first("test", &[1.0, 0.0], 1, 16, None)
        .unwrap();

    assert_eq!(persisted_hnsw::worker_count(), 1);
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
