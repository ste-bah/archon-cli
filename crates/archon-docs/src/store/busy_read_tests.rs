//! Reads must wait for independent SQLite writers, just as guarded writes do.
use std::sync::mpsc;
use std::time::Duration;

fn read_during_write(read: fn(&cozo::DbInstance) -> anyhow::Result<usize>, hold_ms: u64) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("docs.db");
    let db = crate::open_docs_db_for_test(&path).unwrap();
    crate::schema::ensure_doc_schema(&db).unwrap();
    let writer = sqlite::Connection::open(&path).unwrap();
    writer
        .execute("BEGIN EXCLUSIVE; UPDATE cozo SET v = v;")
        .unwrap();
    let (send, receive) = mpsc::channel();
    let (started, start) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        started.send(()).unwrap();
        send.send(read(&db)).unwrap();
    });
    start
        .recv_timeout(Duration::from_secs(5))
        .expect("reader started");
    // The exclusive transaction is held across the first read and multiple retries.
    let early = receive.recv_timeout(Duration::from_millis(hold_ms));
    writer.execute("COMMIT;").unwrap();
    reader.join().unwrap();
    assert!(
        matches!(early, Err(mpsc::RecvTimeoutError::Timeout)),
        "a transient lock escaped instead of waiting: {early:?}"
    );
    assert_eq!(
        receive
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap(),
        0
    );
}

#[test]
fn chunk_list_waits_for_another_handles_exclusive_write() {
    read_during_write(
        |db| super::list_chunks_for_doc(db, "absent").map(|v| v.len()),
        100,
    );
}

#[test]
fn document_list_retries_across_multiple_busy_reads() {
    read_during_write(|db| super::list_doc_sources(db).map(|v| v.len()), 350);
}

#[test]
fn aggregate_read_waits_for_a_long_write_transaction() {
    read_during_write(super::count_chunks, 700);
}
