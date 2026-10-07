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
fn progressing_long_write_is_retried_without_a_total_attempt_limit() {
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
    let mut observed = 0;
    let result = archon_cozo::with_busy_observer(
        move |_, _| {
            // The holder demonstrably completes another write after every busy read.
            observed += 1;
            advance.send(observed < 24).unwrap();
            if observed < 24 {
                progress.recv_timeout(Duration::from_secs(5)).unwrap();
            }
        },
        || super::count_chunks(&db),
    );
    let _ = step.send(false);
    let updates = writer.join().unwrap();
    assert_eq!(
        updates, 23,
        "writer continued beyond the old twenty-attempt cap"
    );
    assert_eq!(result.unwrap(), 1);
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
