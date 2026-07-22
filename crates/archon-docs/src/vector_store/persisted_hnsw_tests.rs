use std::sync::{Mutex, MutexGuard, OnceLock};

use super::*;

fn persisted_hnsw_test_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let lock = LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    persisted_hnsw::clear();
    lock
}

fn wait_for_worker_count(expected: usize) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    while persisted_hnsw::worker_count() != expected && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert_eq!(persisted_hnsw::worker_count(), expected);
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
fn fixed_second_produces_unique_snapshot_basenames() {
    let timestamp = chrono::DateTime::parse_from_rfc3339("2026-07-22T19:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);

    let first = hnsw_dump_basename(timestamp);
    let second = hnsw_dump_basename(timestamp);

    assert_ne!(first, second);
    assert!(first.starts_with("doc-text-20260722T190000Z-"));
    assert!(second.starts_with("doc-text-20260722T190000Z-"));
}

#[test]
fn publishing_new_snapshot_removes_superseded_dump_files() {
    let _lock = persisted_hnsw_test_lock();
    let temp = tempfile::tempdir().unwrap();
    let store = DocVectorStore::open(temp.path()).unwrap();
    write_test_vectors(&store, "test", &[("chunk-a", &[1.0, 0.0])]);
    let first = store.build_hnsw("test", 2, None).unwrap();
    let first_graph = store
        .hnsw_dir("test")
        .join(format!("{}.hnsw.graph", first.dump_basename));
    let first_data = store
        .hnsw_dir("test")
        .join(format!("{}.hnsw.data", first.dump_basename));
    assert!(first_graph.exists());
    assert!(first_data.exists());

    write_test_vectors(&store, "test", &[("chunk-b", &[0.0, 1.0])]);
    let second = store.build_hnsw("test", 2, None).unwrap();
    let published = store.latest_hnsw_manifest("test").unwrap().unwrap();

    assert_ne!(first.dump_basename, second.dump_basename);
    assert_eq!(published.dump_basename, second.dump_basename);
    assert_eq!(published.provider_generation, second.provider_generation);
    assert_eq!(published.vector_count, second.vector_count);
    assert!(!first_graph.exists());
    assert!(!first_data.exists());
    assert!(
        store
            .hnsw_dir("test")
            .join(format!("{}.hnsw.graph", second.dump_basename))
            .exists()
    );
    assert!(
        store
            .hnsw_dir("test")
            .join(format!("{}.hnsw.data", second.dump_basename))
            .exists()
    );

    persisted_hnsw::clear();
    let loads_before = persisted_hnsw_load_count();
    let hits = store
        .search_persisted_first("test", &[0.0, 1.0], 1, 16, None)
        .unwrap();

    assert_eq!(hits[0].chunk_id, "chunk-b");
    assert_eq!(persisted_hnsw_load_count(), loads_before + 1);
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
fn persisted_search_uses_direct_reverse_identifiers() {
    let _lock = persisted_hnsw_test_lock();
    persisted_hnsw::clear();
    let temp = tempfile::tempdir().unwrap();
    let store = DocVectorStore::open(temp.path()).unwrap();
    write_test_vectors(&store, "test", &[("chunk-a", &[1.0, 0.0])]);
    store.build_hnsw("test", 2, None).unwrap();

    let reverse_key = format!("rid/test/{}", hnsw_id("chunk-a"));
    let persisted_chunk_id = store.db.get(reverse_key.as_bytes()).unwrap();
    assert_eq!(persisted_chunk_id.as_deref(), Some(b"chunk-a".as_slice()));

    store.db.delete(id_key("test", "chunk-a")).unwrap();
    let hits = store
        .search_persisted_first("test", &[1.0, 0.0], 1, 16, None)
        .unwrap();

    assert_eq!(hits[0].chunk_id, "chunk-a");
}

#[test]
fn different_snapshot_identities_remain_cached() {
    let _lock = persisted_hnsw_test_lock();
    persisted_hnsw::clear();
    let temp = tempfile::tempdir().unwrap();
    let store = DocVectorStore::open(temp.path()).unwrap();
    write_test_vectors(&store, "first", &[("first-a", &[1.0, 0.0])]);
    write_test_vectors(&store, "second", &[("second-a", &[0.0, 1.0])]);
    store.build_hnsw("first", 2, None).unwrap();
    store.build_hnsw("second", 2, None).unwrap();
    let loads_before = persisted_hnsw_load_count();

    let first_hits = store
        .search_persisted_first("first", &[1.0, 0.0], 1, 16, None)
        .unwrap();
    let second_hits = store
        .search_persisted_first("second", &[0.0, 1.0], 1, 16, None)
        .unwrap();
    let repeated_first_hits = store
        .search_persisted_first("first", &[1.0, 0.0], 1, 16, None)
        .unwrap();

    assert_eq!(first_hits[0].chunk_id, "first-a");
    assert_eq!(second_hits[0].chunk_id, "second-a");
    assert_eq!(repeated_first_hits[0].chunk_id, "first-a");
    assert_eq!(persisted_hnsw_load_count(), loads_before + 2);
}

#[test]
fn stale_provider_does_not_evict_other_cached_snapshot() {
    let _lock = persisted_hnsw_test_lock();
    persisted_hnsw::clear();
    let temp = tempfile::tempdir().unwrap();
    let store = DocVectorStore::open(temp.path()).unwrap();
    write_test_vectors(&store, "first", &[("first-a", &[1.0, 0.0])]);
    write_test_vectors(&store, "second", &[("second-a", &[0.0, 1.0])]);
    store.build_hnsw("first", 2, None).unwrap();
    store.build_hnsw("second", 2, None).unwrap();
    store
        .search_persisted_first("first", &[1.0, 0.0], 1, 16, None)
        .unwrap();
    store
        .search_persisted_first("second", &[0.0, 1.0], 1, 16, None)
        .unwrap();
    let loads_after_initial_searches = persisted_hnsw_load_count();

    write_test_vectors(&store, "second", &[("second-b", &[1.0, 0.0])]);
    store
        .search_persisted_first("second", &[1.0, 0.0], 1, 16, None)
        .unwrap();
    let first_hits = store
        .search_persisted_first("first", &[1.0, 0.0], 1, 16, None)
        .unwrap();

    assert_eq!(first_hits[0].chunk_id, "first-a");
    assert_eq!(persisted_hnsw_load_count(), loads_after_initial_searches);
}

#[test]
fn legacy_forward_identifiers_are_migrated_once() {
    let _lock = persisted_hnsw_test_lock();
    persisted_hnsw::clear();
    let temp = tempfile::tempdir().unwrap();
    let store = DocVectorStore::open(temp.path()).unwrap();
    write_test_vectors(&store, "test", &[("chunk-a", &[1.0, 0.0])]);
    store.build_hnsw("test", 2, None).unwrap();
    let reverse_key = reverse_id_key("test", hnsw_id("chunk-a"));
    store.db.delete(&reverse_key).unwrap();
    store.db.delete(reverse_id_marker_key("test")).unwrap();

    let hits = store
        .search_persisted_first("test", &[1.0, 0.0], 1, 16, None)
        .unwrap();

    assert_eq!(hits[0].chunk_id, "chunk-a");
    assert_eq!(
        store.db.get(reverse_key).unwrap().as_deref(),
        Some(b"chunk-a".as_slice())
    );
}

#[test]
fn first_post_upgrade_write_migrates_all_legacy_identifiers() {
    let _lock = persisted_hnsw_test_lock();
    let temp = tempfile::tempdir().unwrap();
    let store = DocVectorStore::open(temp.path()).unwrap();
    write_test_vectors(&store, "test", &[("legacy", &[1.0, 0.0])]);
    store
        .db
        .delete(reverse_id_key("test", hnsw_id("legacy")))
        .unwrap();
    store.db.delete(reverse_id_marker_key("test")).unwrap();

    write_test_vectors(&store, "test", &[("current", &[0.0, 1.0])]);

    assert_eq!(
        store
            .db
            .get(reverse_id_key("test", hnsw_id("legacy")))
            .unwrap()
            .as_deref(),
        Some(b"legacy".as_slice())
    );
}

#[test]
fn missing_identifier_for_persisted_hit_returns_error() {
    let _lock = persisted_hnsw_test_lock();
    let temp = tempfile::tempdir().unwrap();
    let store = DocVectorStore::open(temp.path()).unwrap();
    write_test_vectors(&store, "test", &[("chunk-a", &[1.0, 0.0])]);
    store.build_hnsw("test", 2, None).unwrap();
    store
        .db
        .delete(reverse_id_key("test", hnsw_id("chunk-a")))
        .unwrap();

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
