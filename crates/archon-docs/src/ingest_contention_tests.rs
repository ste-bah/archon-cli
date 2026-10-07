use crate::models::DocumentStatus;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

fn pause_after_registration(context: &'static str) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("docs.db");
    let db = archon_cozo::open_sqlite_guarded_instance(
        path.to_str().unwrap(),
        "test store",
        archon_cozo::CozoGuardConfig::for_db_path(&path)
            .with_write_lock_wait(Duration::from_millis(20)),
    )
    .unwrap();
    crate::schema::ensure_doc_schema(db.db()).unwrap();
    let progress_db = cozo::DbInstance::new("sqlite", path.to_str().unwrap(), "").unwrap();
    archon_cozo::run_script_guarded(
        &progress_db,
        ":create contention_progress { key: Int => value: Int }",
        Default::default(),
        cozo::ScriptMutability::Mutable,
        "contention progress setup",
        &archon_cozo::CozoGuardConfig::for_db_path(&path),
    )
    .unwrap();
    let sources = temp.path().join("sources");
    std::fs::create_dir(&sources).unwrap();
    std::fs::write(
        sources.join("document.txt"),
        "A complete document survives a contention pause.",
    )
    .unwrap();
    let lock_path = db.config().write_lock_path.clone().unwrap();
    let (make_progress, progress_requested) = mpsc::channel();
    let progress_requested = Arc::new(Mutex::new(Some(progress_requested)));
    let (ready, started) = mpsc::channel();
    let holder = Arc::new(Mutex::new(None));
    let holder_thread = Arc::clone(&holder);
    let mut armed = false;
    let (paused, evidence) = mpsc::channel();
    let mut pauses = 0;
    let write_path = path.clone();
    let result = archon_cozo::with_guarded_failure(
        move |actual| {
            if actual == context && !armed {
                armed = true;
                let lock_path = lock_path.clone();
                let progress_db = progress_db.clone();
                let write_path = write_path.clone();
                let progress_requested = progress_requested.lock().unwrap().take().unwrap();
                let ready = ready.clone();
                let thread = std::thread::spawn(move || {
                    archon_cozo::with_write_lock_blocking_timeout(
                        &lock_path,
                        "competing docs writer",
                        Duration::from_secs(2),
                        || {
                            ready.send(()).unwrap();
                            progress_requested.recv_timeout(Duration::from_secs(10)).unwrap();
                            for value in 0..3 {
                                archon_cozo::run_script_guarded(
                                    &progress_db,
                                    &format!("?[key, value] <- [[1, {value}]] :put contention_progress {{key => value}}"),
                                    Default::default(),
                                    cozo::ScriptMutability::Mutable,
                                    "competing docs writer progress",
                                    &archon_cozo::CozoGuardConfig::for_db_path(&write_path),
                                )
                                .unwrap();
                                std::thread::sleep(Duration::from_millis(5));
                            }
                            Ok(())
                        },
                    )
                    .unwrap();
                });
                *holder_thread.lock().unwrap() = Some(thread);
                started.recv_timeout(Duration::from_secs(10)).unwrap();
            }
            None
        },
        || {
            archon_cozo::with_busy_observer(
                move |_, error| {
                    if error.contains("retryable store busy") {
                        pauses += 1;
                        paused.send(()).unwrap();
                        if pauses == 1 {
                            let _ = make_progress.send(());
                        }
                    }
                },
                || {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .unwrap()
                        .block_on(crate::ingest::ingest_directory(db.db(), &sources))
                },
            )
        },
    );
    holder
        .lock()
        .unwrap()
        .take()
        .expect("real lock holder ran")
        .join()
        .unwrap();
    assert!(
        evidence.try_iter().count() >= 1,
        "production ingestion did not resume real acquisition pauses: {result:?}"
    );
    let result = result.unwrap();
    assert_eq!(result.sources_registered, 1);
    assert_eq!(result.sources_failed, 0);
    let docs = crate::store::list_doc_sources(db.db()).unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].status, DocumentStatus::Ingested);
    assert!(
        !crate::store::list_chunks_for_doc(db.db(), &docs[0].document_id)
            .unwrap()
            .is_empty()
    );
    assert!(
        !crate::store::list_pages_for_doc(db.db(), &docs[0].document_id)
            .unwrap()
            .is_empty()
    );
    let retry = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(crate::ingest::ingest_directory(db.db(), &sources))
        .unwrap();
    assert_eq!(retry.sources_skipped_duplicate, 1);
    assert_eq!(crate::store::list_doc_sources(db.db()).unwrap().len(), 1);
}
#[test]
fn directory_ingestion_resumes_a_pause_at_job_creation() {
    pause_after_registration("insert doc_processing_jobs");
}
#[test]
fn directory_ingestion_resumes_a_pause_at_page_creation() {
    pause_after_registration("insert doc_pages");
}
#[test]
fn directory_ingestion_resumes_a_pause_at_chunk_creation() {
    pause_after_registration("insert doc_chunks");
}
