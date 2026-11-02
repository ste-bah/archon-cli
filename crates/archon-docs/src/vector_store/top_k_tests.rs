//! Regression tests for #545: a fresh `hnsw_rs` graph can return fewer than
//! `top_k` hits although it holds at least `top_k` points. Layer assignment is
//! random per build (OS-seeded), so each test searches many fresh graphs.

use super::*;

/// Fresh graphs per test. One graph in ~32 drops a hit on the base code, so
/// 1000 graphs leave a base pass chance near (31/32)^1000, about 1e-14.
const FRESH_GRAPHS: usize = 1000;

fn write_two_close_vectors(store: &DocVectorStore) {
    let rows = [
        VectorWrite {
            chunk_id: "chunk-a",
            content_hash: "hash-a",
            provider: "test",
            embedding: &[1.0, 0.0],
        },
        VectorWrite {
            chunk_id: "chunk-b",
            content_hash: "hash-b",
            provider: "test",
            embedding: &[1.0, 1.0 / 64.0],
        },
    ];
    store.put_vectors(&rows).unwrap();
}

#[test]
fn in_memory_search_returns_min_of_top_k_and_count_for_every_fresh_graph() {
    let temp = tempfile::tempdir().unwrap();
    let store = DocVectorStore::open(temp.path()).unwrap();
    write_two_close_vectors(&store);

    for graph in 0..FRESH_GRAPHS {
        let hits = store
            .search_in_memory("test", &[1.0, 0.0], 3, 16, None)
            .unwrap();
        let chunk_ids: Vec<_> = hits.iter().map(|hit| hit.chunk_id.as_str()).collect();
        assert_eq!(chunk_ids, ["chunk-a", "chunk-b"], "fresh graph {graph}");
    }
}

#[test]
fn persisted_search_returns_min_of_top_k_and_count_for_every_fresh_graph() {
    let _lock = hnsw_state_guard();
    let temp = tempfile::tempdir().unwrap();
    let store = DocVectorStore::open(temp.path()).unwrap();
    write_two_close_vectors(&store);
    let loads_before = persisted_hnsw_load_count();

    for graph in 0..FRESH_GRAPHS {
        store.build_hnsw("test", 2, None).unwrap();
        let hits = store
            .search_persisted_first("test", &[1.0, 0.0], 3, 16, None)
            .unwrap();
        let chunk_ids: Vec<_> = hits.iter().map(|hit| hit.chunk_id.as_str()).collect();
        assert_eq!(chunk_ids, ["chunk-a", "chunk-b"], "fresh graph {graph}");
    }
    // Every search read its own persisted snapshot, not the in-memory path.
    assert_eq!(persisted_hnsw_load_count() - loads_before, FRESH_GRAPHS);
}
