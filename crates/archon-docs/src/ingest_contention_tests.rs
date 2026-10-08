//! Production document-store settings under real lock and SQLite contention.
//!
//! Every store here is opened with the production guard config
//! (`docs_db_cache::guard_config`), never a test-only wait: ingestion waits
//! while a real holder makes progress, and a wedged holder ends the wait as a
//! typed pause after the production no-progress window.
use crate::models::DocumentStatus;
use archon_cozo::{DEFAULT_WRITE_LOCK_WAIT, StoreBusy};
use std::path::Path;
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

fn ingest(
    db: &cozo::DbInstance,
    dir: &Path,
) -> anyhow::Result<crate::ingest_directory::IngestResult> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(crate::ingest::ingest_directory(db, dir))
}

fn sources(temp: &tempfile::TempDir) -> std::path::PathBuf {
    let sources = temp.path().join("sources");
    std::fs::create_dir(&sources).unwrap();
    std::fs::write(
        sources.join("document.txt"),
        "A complete document survives a contended store.",
    )
    .unwrap();
    sources
}

fn assert_ingested_once(db: &cozo::DbInstance, dir: &Path) {
    let docs = crate::store::list_doc_sources(db).unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].status, DocumentStatus::Ingested);
    let id = &docs[0].document_id;
    assert!(
        !crate::store::list_chunks_for_doc(db, id)
            .unwrap()
            .is_empty()
    );
    assert!(!crate::store::list_pages_for_doc(db, id).unwrap().is_empty());
    assert_eq!(ingest(db, dir).unwrap().sources_skipped_duplicate, 1);
}

/// At `context`, a real holder takes the store's write lock and commits a
/// write every 100ms for 1.5s. The statement must queue behind it and finish.
fn waits_through_a_progressing_holder(context: &'static str) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("docs.db");
    let db = crate::open_docs_db_for_test(&path).unwrap();
    crate::schema::ensure_doc_schema(db.db()).unwrap();
    let writer = crate::open_docs_db_for_test(&path).unwrap();
    archon_cozo::run_script_guarded(
        writer.db(),
        ":create contention_progress { key: Int => value: Int }",
        Default::default(),
        cozo::ScriptMutability::Mutable,
        "contention progress setup",
        writer.config(),
    )
    .unwrap();
    let dir = sources(&temp);
    let lock = db.config().write_lock_path.clone().unwrap();
    let holder = Arc::new(Mutex::new(None));
    let holder_slot = Arc::clone(&holder);
    let mut writer = Some(writer);
    let (waited, evidence) = mpsc::channel();
    let result = archon_cozo::with_guarded_failure(
        move |actual| {
            if actual == context
                && let Some(writer) = writer.take()
            {
                let (ready, started) = mpsc::channel();
                let lock = lock.clone();
                *holder_slot.lock().unwrap() = Some(std::thread::spawn(move || {
                    archon_cozo::with_write_lock_blocking(&lock, "competing docs writer", || {
                        ready.send(()).unwrap();
                        for value in 0..15 {
                            archon_cozo::run_script_guarded(
                                writer.db(),
                                &format!("?[key, value] <- [[1, {value}]] :put contention_progress {{key => value}}"),
                                Default::default(),
                                cozo::ScriptMutability::Mutable,
                                "competing docs writer progress",
                                writer.config(),
                            )?;
                            std::thread::sleep(Duration::from_millis(100));
                        }
                        Ok(())
                    })
                    .unwrap();
                }));
                started.recv_timeout(Duration::from_secs(30)).unwrap();
            }
            None
        },
        || {
            archon_cozo::with_busy_observer(
                move |actual, message| {
                    if actual == context && message.contains("waiting for writer progress") {
                        let _ = waited.send(());
                    }
                },
                || ingest(db.db(), &dir),
            )
        },
    );
    holder
        .lock()
        .unwrap()
        .take()
        .expect("holder ran")
        .join()
        .unwrap();
    assert!(
        evidence.try_iter().count() >= 1,
        "the statement never queued behind the real holder: {result:?}"
    );
    let result = result.unwrap();
    assert_eq!((result.sources_registered, result.sources_failed), (1, 0));
    assert_ingested_once(db.db(), &dir);
}

#[test]
fn production_ingestion_waits_through_a_progressing_holder_at_job_creation() {
    waits_through_a_progressing_holder("insert doc_processing_jobs");
}

#[test]
fn production_ingestion_waits_through_a_progressing_holder_at_page_creation() {
    waits_through_a_progressing_holder("insert doc_pages");
}

#[test]
fn production_ingestion_waits_through_a_progressing_holder_at_chunk_creation() {
    waits_through_a_progressing_holder("insert doc_chunks");
}

/// A correct wait returns about one production window after the last
/// progress; a wait without a progress check never returns.
const GIVE_UP: Duration = Duration::from_secs(150);

#[test]
fn production_ingestion_pauses_on_a_wedged_lock_holder_then_resumes() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("docs.db");
    let db = crate::open_docs_db_for_test(&path).unwrap();
    crate::schema::ensure_doc_schema(db.db()).unwrap();
    let dir = sources(&temp);
    let lock = db.config().write_lock_path.clone().unwrap();
    let (done, finished) = mpsc::channel();
    let worker_dir = dir.clone();
    let worker_db = crate::open_docs_db_for_test(&path).unwrap();
    // A raw OS lock outside the guard's bookkeeping, as another process holds it.
    let mut held = archon_cozo::OwnerLockFile::open(&lock).unwrap();
    let guard = held.try_own().unwrap().expect("the store lock is free");
    let started = Instant::now();
    std::thread::spawn(move || {
        let _ = done.send((ingest(worker_db.db(), &worker_dir), Instant::now()));
    });
    let (paused, returned) = finished
        .recv_timeout(GIVE_UP)
        .expect("ingestion never paused while the holder was wedged");
    drop(guard);
    let error = paused.expect_err("a wedged holder pauses the run");
    let busy = StoreBusy::find(error.as_ref()).expect("the pause stays typed StoreBusy");
    assert!(busy.detail.contains(&lock.display().to_string()), "{busy}");
    assert!(returned.duration_since(started) >= DEFAULT_WRITE_LOCK_WAIT);
    let resumed = ingest(db.db(), &dir).unwrap();
    assert_eq!((resumed.sources_registered, resumed.sources_failed), (1, 0));
    assert_ingested_once(db.db(), &dir);
}

#[test]
fn production_read_pauses_on_a_wedged_sqlite_writer() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("docs.db");
    let db = crate::open_docs_db_for_test(&path).unwrap();
    crate::schema::ensure_doc_schema(db.db()).unwrap();
    let writer = sqlite::Connection::open(&path).unwrap();
    writer
        .execute("BEGIN EXCLUSIVE; UPDATE cozo SET v = v;")
        .unwrap();
    let (done, finished) = mpsc::channel();
    let started = Instant::now();
    std::thread::spawn(move || {
        let _ = done.send((crate::store::list_doc_sources(db.db()), Instant::now()));
    });
    let outcome = finished.recv_timeout(GIVE_UP);
    writer.execute("COMMIT;").unwrap();
    let (result, returned) = outcome.expect("a busy read retried with no progress limit");
    let error = result.expect_err("a wedged writer pauses the read");
    let busy = StoreBusy::find(error.as_ref()).expect("the pause stays typed StoreBusy");
    assert!(
        busy.detail.contains("docs.db.archon-cozo-write.lock"),
        "{busy}"
    );
    assert!(busy.detail.contains("locked"), "{busy}");
    assert!(returned.duration_since(started) >= DEFAULT_WRITE_LOCK_WAIT);
}
