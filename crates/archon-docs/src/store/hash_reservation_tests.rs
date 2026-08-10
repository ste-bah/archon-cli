//! Persisted concurrency regression coverage for document-hash reservations.
//!
//! `doc_sources` is keyed on `document_id`, and every ingest mints a fresh UUID,
//! so nothing in the schema stops two documents carrying one `content_hash`.
//! Dedup is a read followed by a write, and it only holds while nothing else can
//! write between the two — which is exactly what two `DbInstance` handles on one
//! file can do. That is the shipping counterpart of the KB defect in #111, and
//! these tests are the measurement.

use std::sync::{Arc, Barrier};
use std::time::Duration;

use super::hash_reservation_test_hooks::{
    ReservationRendezvous, pause_before_reservation_for_tests,
};
use crate::ingest_text::ingest_text_source;

/// How long an ingest waits inside its reservation for the other to join.
///
/// Long enough that an unserialised peer — milliseconds away — always arrives,
/// so an unlocked reservation reproduces the duplicate every run; short enough
/// that the serialised path, where the peer is parked on the write lock and
/// never arrives, costs one such wait per run.
const RESERVATION_RENDEZVOUS_TIMEOUT: Duration = Duration::from_millis(500);

const SHARED_CONTENT: &str = "Two handles, one document.";

fn shared_content_hash() -> String {
    crate::hash::sha256_str(SHARED_CONTENT)
}

/// Two ingests of identical content, on independent handles to one file.
///
/// The handles are deliberately not shared: `acquire_docs_db` hands one
/// `Arc<DbInstance>` per canonical path *within a process*, so a shared handle
/// would prove nothing about the case that actually occurs — a second `archon`
/// process, or any caller that opens the store itself.
#[test]
fn concurrent_handles_register_one_document_for_one_content_hash() {
    let temp = tempfile::tempdir().expect("temp database directory");
    let path = temp.path().join("docs.db");

    let hash = shared_content_hash();
    let rendezvous = ReservationRendezvous::new(2, RESERVATION_RENDEZVOUS_TIMEOUT);
    let _pause = pause_before_reservation_for_tests(&hash, Arc::clone(&rendezvous));

    let start = Arc::new(Barrier::new(2));
    let first = spawn_ingest(&path, Arc::clone(&start), "https://example.test/a.txt");
    let second = spawn_ingest(&path, start, "https://example.test/b.txt");
    let first = first.join().expect("first ingest thread");
    let second = second.join().expect("second ingest thread");

    // The cause, measured where it happens. A peak of 2 means both ingests held
    // a read taken before either wrote, which is what lets each conclude the
    // hash was unclaimed.
    assert_eq!(
        rendezvous.peak_in_flight(),
        1,
        "content-hash reservations must not overlap across DbInstance handles"
    );
    assert!(
        rendezvous.arrivals() >= 1,
        "at least one ingest must reach the reservation"
    );

    // The symptom.
    assert_eq!(
        first.document_id, second.document_id,
        "both ingests must resolve to one document"
    );
    assert_eq!(
        usize::from(first.was_new) + usize::from(second.was_new),
        1,
        "exactly one ingest may register the shared content hash"
    );

    let reopened = crate::open_docs_db_for_test(&path).expect("reopen document store");
    let documents = super::list_doc_sources(&reopened).expect("list persisted documents");
    assert_eq!(
        documents.len(),
        1,
        "one content hash must leave one row in doc_sources, found {documents:#?}"
    );
    assert_eq!(documents[0].content_hash, hash);
    println!(
        "EVIDENCE docs_concurrent_reservation documents={} arrivals={} peak_in_flight={}",
        documents.len(),
        rendezvous.arrivals(),
        rendezvous.peak_in_flight()
    );
}

/// Ingest nested inside an already-guarded mutable operation on the same store.
///
/// The reservation lock is an OS byte-range lock, and on Windows those conflict
/// between handles inside one process — so an ingest that re-acquires a lock its
/// own thread already holds through `run_guarded` would block on itself. A
/// watchdog rather than a plain call: a regression here hangs, and a hung job is
/// far worse to diagnose in CI than a failed assertion.
#[test]
fn reservation_inside_a_guarded_operation_does_not_self_deadlock() {
    let temp = tempfile::tempdir().expect("temp database directory");
    let path = temp.path().join("docs.db");
    let worker_path = path.clone();

    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let outcome = (|| {
            let db = crate::open_docs_db_for_test(&worker_path)?;
            let config = archon_cozo::CozoGuardConfig::for_db_path(&worker_path);
            archon_cozo::run_guarded(
                "docs ingest inside a guarded operation",
                cozo::ScriptMutability::Mutable,
                &config,
                || {
                    ingest_text_source(
                        &db,
                        "https://example.test/nested.txt",
                        "text/plain",
                        "Nested reservation content.",
                    )
                    .map_err(anyhow::Error::from)
                },
            )
        })();
        let _ = sender.send(outcome.map(|result| result.was_new));
    });

    let was_new = receiver
        .recv_timeout(Duration::from_secs(20))
        .expect("nested ingest deadlocked on the reservation lock")
        .expect("nested ingest succeeds");

    assert!(was_new);
    let reopened = crate::open_docs_db_for_test(&path).expect("reopen document store");
    assert_eq!(
        super::list_doc_sources(&reopened)
            .expect("list persisted documents")
            .len(),
        1
    );
}

fn spawn_ingest(
    path: &std::path::Path,
    start: Arc<Barrier>,
    source: &'static str,
) -> std::thread::JoinHandle<crate::ingest_text::IngestTextSourceResult> {
    let path = path.to_path_buf();
    std::thread::spawn(move || {
        // Each thread opens its own guarded instance: two genuinely independent
        // `DbInstance` handles on one file, which is the whole point.
        let db = crate::open_docs_db_for_test(&path).expect("open document store");
        start.wait();
        ingest_text_source(&db, source, "text/plain", SHARED_CONTENT).expect("ingest succeeds")
    })
}
