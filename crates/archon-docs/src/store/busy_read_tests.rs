//! Commit only after evidence that the actual read statement encountered SQLITE_BUSY.
use std::sync::mpsc;
use std::time::Duration;

fn read_during_write(read: fn(&cozo::DbInstance) -> anyhow::Result<usize>, retries: usize) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("docs.db");
    let db = crate::open_docs_db_for_test(&path).unwrap();
    seed(&db);
    let writer = sqlite::Connection::open(&path).unwrap();
    writer
        .execute("BEGIN EXCLUSIVE; UPDATE cozo SET v = v;")
        .unwrap();
    let (send, receive) = mpsc::channel();
    let (busy, evidence) = mpsc::channel();
    let (resume, resumed) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        archon_cozo::with_busy_observer(
            move |_, error| {
                assert!(
                    error.contains("locked") || error.contains("SQLITE_BUSY"),
                    "{error}"
                );
                busy.send(()).unwrap();
                resumed.recv().unwrap();
            },
            || send.send(read(&db)).unwrap(),
        );
    });
    let mut observed = 0;
    for index in 0..retries {
        if evidence.recv_timeout(Duration::from_secs(5)).is_err() {
            break;
        }
        observed += 1;
        if index + 1 < retries {
            resume.send(()).unwrap();
        }
    }
    // On both success and a missing-observer failure, release the writer and
    // reader before asserting so an old unguarded path cannot strand a thread.
    writer.execute("COMMIT;").unwrap();
    if observed != 0 {
        resume.send(()).unwrap();
    }
    reader.join().unwrap();
    assert_eq!(
        observed, retries,
        "no proof that the actual statement encountered contention"
    );
    assert_eq!(
        receive
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap(),
        1
    );
}

#[test]
fn chunk_list_waits_for_another_handles_exclusive_write() {
    read_during_write(
        |db| super::list_chunks_for_doc(db, "read-fixture").map(|v| v.len()),
        1,
    );
}
#[test]
fn document_list_retries_across_multiple_busy_reads() {
    read_during_write(|db| super::list_doc_sources(db).map(|v| v.len()), 2);
}
#[test]
fn aggregate_read_waits_for_a_long_write_transaction() {
    read_during_write(super::count_chunks, 3);
}

#[test]
fn lock_held_beyond_default_window_is_explicitly_retryable_busy() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("docs.db");
    let db = crate::open_docs_db_for_test(&path).unwrap();
    seed(&db);
    let (ready, started) = mpsc::channel();
    let (step, requests) = mpsc::channel::<bool>();
    let (done, progress) = mpsc::channel();
    let writer = std::thread::spawn(move || {
        let connection = sqlite::Connection::open(&path).unwrap();
        connection
            .execute("BEGIN EXCLUSIVE; UPDATE cozo SET v = v;")
            .unwrap();
        ready.send(()).unwrap();
        let mut updates = 0;
        while let Ok(update) = requests.recv() {
            if !update {
                connection.execute("COMMIT;").unwrap();
                break;
            }
            connection.execute("UPDATE cozo SET v = v;").unwrap();
            updates += 1;
            done.send(()).unwrap();
        }
        updates
    });
    started.recv_timeout(Duration::from_secs(5)).unwrap();
    let advance = step.clone();
    let error = archon_cozo::with_busy_observer(
        move |_, _| {
            // The holder demonstrably completes another write after every busy read.
            advance.send(true).unwrap();
            progress.recv_timeout(Duration::from_secs(5)).unwrap();
        },
        || super::count_chunks(&db),
    )
    .unwrap_err();
    // The write lock stays held through all twenty attempts (~19s). Exhaustion
    // must identify an incomplete operation the caller can retry, never zero rows.
    let message = error.to_string();
    step.send(false).unwrap();
    assert_eq!(
        writer.join().unwrap(),
        20,
        "every busy attempt must observe holder progress"
    );
    assert!(
        message.contains("retryable store busy after 20 attempts"),
        "{message}"
    );
    assert!(archon_cozo::is_retryable_cozo_error(&message));
    assert_eq!(super::count_chunks(&db).unwrap(), 1);
}

fn seed(db: &cozo::DbInstance) {
    use crate::models::{ChunkArtifact, DocumentStatus, SourceDocument};
    crate::schema::ensure_doc_schema(db).unwrap();
    super::insert_doc_source(
        db,
        &SourceDocument {
            document_id: "read-fixture".into(),
            source_path: "/fixture.txt".into(),
            media_type: "text/plain".into(),
            content_hash: "hash".into(),
            discovered_at: "2026-01-01T00:00:00Z".into(),
            status: DocumentStatus::Discovered,
        },
    )
    .unwrap();
    super::insert_chunk(
        db,
        &ChunkArtifact {
            chunk_id: "chunk".into(),
            document_id: "read-fixture".into(),
            artifact_id: "artifact".into(),
            chunk_index: 0,
            page_start: 1,
            page_end: 1,
            content: "fixture".into(),
            content_hash: "hash".into(),
            embedding_status: "pending".into(),
        },
    )
    .unwrap();
}
