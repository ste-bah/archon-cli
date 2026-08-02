//! Persisted concurrency regression coverage for KB chunk reservations.

use std::collections::BTreeMap;
use std::sync::{Arc, Barrier};
use std::time::Duration;

use cozo::{DataValue, DbInstance, ScriptMutability};

use crate::kb::ingest_storage::{ChunkData, ChunkStorage};
use crate::kb::ingest_storage_test_hooks::ReservationRendezvous;
use crate::kb::schema::{ensure_kb_embedding_schema, ensure_kb_schema};

/// How long an ingest thread waits inside its reservation for the other to join.
///
/// Long enough that an unserialised peer — milliseconds away — always arrives,
/// so the defect still reproduces; short enough that the serialised path, where
/// the peer is parked on the write lock and never arrives, costs one such wait
/// per run.
const RESERVATION_RENDEZVOUS_TIMEOUT: Duration = Duration::from_millis(500);

fn content_hash(content: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(content.as_bytes()))
}

fn sqlite_db(path: &str) -> DbInstance {
    DbInstance::new("sqlite", path, "").expect("open persisted Cozo database")
}

fn count_rows(db: &DbInstance, relation: &str, key: &str) -> usize {
    let result = db
        .run_script(
            &format!("?[count({key})] := *{relation}{{{key}}}"),
            BTreeMap::new(),
            ScriptMutability::Immutable,
        )
        .expect("count persisted rows");
    result.rows[0][0].get_int().unwrap_or_default() as usize
}

fn content_hash_owners(db: &DbInstance) -> Vec<(String, String)> {
    db.run_script(
        "?[content_hash, node_id] := *kb_content_hashes{content_hash, node_id}",
        BTreeMap::new(),
        ScriptMutability::Immutable,
    )
    .expect("read persisted hash owners")
    .rows
    .into_iter()
    .map(|row| {
        (
            row[0].get_str().unwrap_or_default().to_owned(),
            row[1].get_str().unwrap_or_default().to_owned(),
        )
    })
    .collect()
}

#[test]
fn generic_reservation_error_is_returned_unchanged() {
    let storage = storage_with_schema();
    let _failure = storage.fail_hash_reservation_for_tests(
        "reserve content hashes failed: injected reservation outage",
        None,
    );

    let error = store_one(&storage, "content").expect_err("injected failure returns error");

    assert_eq!(
        error.to_string(),
        "reserve content hashes failed: injected reservation outage"
    );
}

#[test]
fn injected_reservation_conflict_reconciles_physical_hash_owner() {
    let temp = tempfile::tempdir().expect("temp database directory");
    let path = temp.path().join("kb.sqlite");
    let path = path.to_string_lossy().into_owned();
    initialize_persisted_kb(&path);
    let storage = ChunkStorage::new(sqlite_db(&path));
    let competing_writer = sqlite_db(&path);
    let hook = storage.persist_conflict_then_fail_hash_reservation_for_tests(move |hash| {
        persist_competing_node(&competing_writer, &hash)
    });

    let result = store_one(&storage, "content").expect("physical conflict reconciles");

    assert!(hook.was_consumed());
    assert_eq!(result.nodes_created, 0);
    assert_eq!(result.chunks_processed, 1);
    let owners = content_hash_owners(storage.db());
    assert_eq!(owners.len(), 1);
    assert_node_exists(storage.db(), &owners[0].0, &owners[0].1);
}

fn storage_with_schema() -> ChunkStorage {
    let db = DbInstance::new("mem", "", "").expect("open memory Cozo database");
    ensure_kb_schema(&db).expect("initialize KB schema");
    ChunkStorage::new(db)
}

fn store_one(storage: &ChunkStorage, content: &str) -> anyhow::Result<crate::kb::IngestResult> {
    storage.store(
        &[ChunkData {
            title: "test".into(),
            content: content.into(),
        }],
        None,
        "test-source",
        "test",
        content_hash,
    )
}

fn persist_competing_node(db: &DbInstance, hash: &str) {
    let node_id = "competing-node";
    let mut params = BTreeMap::new();
    params.insert("hash".to_string(), DataValue::from(hash));
    params.insert("node_id".to_string(), DataValue::from(node_id));
    db.run_script(
        "?[content_hash, node_id] <- [[$hash, $node_id]]\n:put kb_content_hashes { content_hash => node_id }",
        params,
        ScriptMutability::Mutable,
    )
    .expect("persist competing hash");
    let mut params = BTreeMap::new();
    params.insert("hash".to_string(), DataValue::from(hash));
    params.insert("node_id".to_string(), DataValue::from(node_id));
    db.run_script(
        "?[node_id, node_type, source, domain_tag, title, content, content_hash, chunk_index, created_at, updated_at] <- [[$node_id, 'raw', 'race', 'test', 'race', 'content', $hash, 0, 0.0, 0.0]]\n:put kb_nodes { node_id => node_type, source, domain_tag, title, content, content_hash, chunk_index, created_at, updated_at }",
        params,
        ScriptMutability::Mutable,
    )
    .expect("persist competing node");
}

fn assert_node_exists(db: &DbInstance, hash: &str, node_id: &str) {
    let mut params = BTreeMap::new();
    params.insert("node_id".to_owned(), DataValue::from(node_id));
    let node = db
        .run_script(
            "?[content_hash] := *kb_nodes{node_id, content_hash}, node_id = $node_id",
            params,
            ScriptMutability::Immutable,
        )
        .expect("read persisted node");
    assert_eq!(node.rows[0][0].get_str(), Some(hash));
}

#[test]
fn concurrent_sqlite_storages_dedupe_shared_hash_and_preserve_unique_embeddings() {
    let temp = tempfile::tempdir().expect("temp database directory");
    let path = temp.path().join("kb.sqlite");
    let path = path.to_string_lossy().into_owned();
    initialize_persisted_kb(&path);

    let (first_created, second_created) = concurrent_nodes_created(&path);

    assert_eq!(first_created + second_created, 3);
    let state = assert_reopened_integrity(&path);
    println!(
        "EVIDENCE kb_concurrent first_created={first_created} second_created={second_created} nodes={} embeddings={} hashes={} orphan_hashes=0 orphan_embeddings=0",
        state.nodes, state.embeddings, state.hashes
    );
}

/// Ingest nested inside a guarded mutable operation on the same database.
///
/// The reservation lock is an OS byte-range lock, and on Windows those conflict
/// between handles inside one process — so an ingest that re-acquires a lock its
/// own thread already holds through `run_guarded` would block on itself. A
/// watchdog rather than a plain call: a regression here hangs, and a hung job is
/// far worse to diagnose in CI than a failed assertion.
#[test]
fn ingest_inside_a_guarded_operation_does_not_self_deadlock() {
    let temp = tempfile::tempdir().expect("temp database directory");
    let path = temp.path().join("kb.sqlite");
    let path = path.to_string_lossy().into_owned();
    initialize_persisted_kb(&path);

    let (sender, receiver) = std::sync::mpsc::channel();
    let worker_path = path.clone();
    std::thread::spawn(move || {
        let storage = ChunkStorage::for_db_path(sqlite_db(&worker_path), &worker_path);
        let config = archon_cozo::CozoGuardConfig::for_db_path(&worker_path);
        let nested = archon_cozo::run_guarded(
            "kb ingest inside a guarded operation",
            ScriptMutability::Mutable,
            &config,
            || store_one(&storage, "nested-content"),
        );
        let _ = sender.send(nested.map(|result| result.nodes_created));
    });

    let nodes_created = receiver
        .recv_timeout(std::time::Duration::from_secs(20))
        .expect("nested ingest deadlocked on the reservation lock")
        .expect("nested ingest succeeds");

    assert_eq!(nodes_created, 1);
    let reopened = sqlite_db(&path);
    assert_eq!(count_rows(&reopened, "kb_nodes", "node_id"), 1);
    assert_eq!(content_hash_owners(&reopened).len(), 1);
}

fn initialize_persisted_kb(path: &str) {
    let setup = sqlite_db(path);
    ensure_kb_schema(&setup).expect("initialize KB schema");
    ensure_kb_embedding_schema(&setup, "test", 2, None).expect("initialize embedding schema");
}

fn concurrent_nodes_created(path: &str) -> (usize, usize) {
    // Two genuinely independent handles on one file, which is the whole point:
    // their reservations must serialise on the database's write lock, not on a
    // shared `Arc<DbInstance>`.
    let first = Arc::new(ChunkStorage::for_db_path(sqlite_db(path), path));
    let second = Arc::new(ChunkStorage::for_db_path(sqlite_db(path), path));
    let reservation = ReservationRendezvous::new(2, RESERVATION_RENDEZVOUS_TIMEOUT);
    let _first_pause = first.pause_before_hash_reservation_for_tests(Arc::clone(&reservation));
    let _second_pause = second.pause_before_hash_reservation_for_tests(Arc::clone(&reservation));
    let start = Arc::new(Barrier::new(2));
    let first_task = spawn_store(first, start.clone(), first_chunks(), "first-source");
    let second_task = spawn_store(second, start, second_chunks(), "second-source");
    let first = first_task.join().expect("first storage thread");
    let second = second_task.join().expect("second storage thread");
    // The node counts below are the symptom; this is the cause. Both threads
    // must reach the point between their read and their reservation, and they
    // must never be there together — that overlap is what let two writers each
    // conclude the shared hash was unclaimed.
    assert_eq!(
        reservation.arrivals(),
        2,
        "both ingest threads must reach the reservation"
    );
    assert_eq!(
        reservation.peak_in_flight(),
        1,
        "content-hash reservations must not overlap across DbInstance handles"
    );
    (
        first.expect("first store succeeds").nodes_created,
        second.expect("second store succeeds").nodes_created,
    )
}

fn spawn_store(
    storage: Arc<ChunkStorage>,
    start: Arc<Barrier>,
    chunks: [ChunkData; 2],
    source: &'static str,
) -> std::thread::JoinHandle<anyhow::Result<crate::kb::IngestResult>> {
    std::thread::spawn(move || {
        start.wait();
        storage.store(
            &chunks,
            Some(&[vec![1.0, 0.0], vec![0.0, 1.0]]),
            source,
            "test",
            content_hash,
        )
    })
}

fn first_chunks() -> [ChunkData; 2] {
    [
        ChunkData {
            title: "first".into(),
            content: "first-only".into(),
        },
        ChunkData {
            title: "shared".into(),
            content: "shared-content".into(),
        },
    ]
}

fn second_chunks() -> [ChunkData; 2] {
    [
        ChunkData {
            title: "shared".into(),
            content: "shared-content".into(),
        },
        ChunkData {
            title: "second".into(),
            content: "second-only".into(),
        },
    ]
}

struct ReopenedIntegrity {
    nodes: usize,
    embeddings: usize,
    hashes: usize,
}

fn assert_reopened_integrity(path: &str) -> ReopenedIntegrity {
    let reopened = sqlite_db(path);
    let state = ReopenedIntegrity {
        hashes: content_hash_owners(&reopened).len(),
        nodes: count_rows(&reopened, "kb_nodes", "node_id"),
        embeddings: count_rows(&reopened, "kb_embeddings", "node_id"),
    };
    assert_eq!(state.hashes, 3);
    assert_eq!(state.nodes, 3);
    assert_eq!(state.embeddings, 3);
    let expected = expected_raw_ownership();
    let actual = raw_ownership(&reopened);
    assert_eq!(actual.len(), expected.len());
    assert!(
        actual
            .iter()
            .all(|(hash, _, content)| expected.contains(&(hash.clone(), content.clone())))
    );
    assert_no_orphans(&reopened);
    state
}

fn expected_raw_ownership() -> std::collections::BTreeSet<(String, String)> {
    ["first-only", "shared-content", "second-only"]
        .into_iter()
        .map(|content| (content_hash(content), content.to_owned()))
        .collect()
}

fn raw_ownership(db: &DbInstance) -> Vec<(String, String, String)> {
    db.run_script(
        "?[hash, node_id, content] := *kb_content_hashes{content_hash: hash, node_id}, \
         *kb_nodes{node_id, content_hash: hash, content, node_type}, node_type = 'raw'",
        BTreeMap::new(),
        ScriptMutability::Immutable,
    )
    .expect("read exact raw ownership")
    .rows
    .into_iter()
    .map(|row| {
        (
            row[0].get_str().unwrap_or_default().to_owned(),
            row[1].get_str().unwrap_or_default().to_owned(),
            row[2].get_str().unwrap_or_default().to_owned(),
        )
    })
    .collect()
}

fn assert_no_orphans(db: &DbInstance) {
    for (query, message) in [
        (
            "?[hash] := *kb_content_hashes{content_hash: hash, node_id}, not *kb_nodes{node_id}",
            "hash owner must exist",
        ),
        (
            "?[node] := *kb_embeddings{node_id: node}, not *kb_nodes{node_id: node}",
            "embedding owner must exist",
        ),
        (
            "?[node] := *kb_nodes{node_id: node, node_type}, node_type = 'raw', not *kb_content_hashes{content_hash, node_id: node}",
            "raw node must have a hash owner",
        ),
    ] {
        let rows = db
            .run_script(query, BTreeMap::new(), ScriptMutability::Immutable)
            .expect("check persisted orphan rows");
        assert!(rows.rows.is_empty(), "{message}");
    }
}
