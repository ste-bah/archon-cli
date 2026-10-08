//! A registration that a store pause interrupted resumes on the next run.
//!
//! A pause (`StoreBusy`) can arrive after the content hash is registered. The
//! next run must finish that document, not skip it as a duplicate of itself.
//! A live ingest of the same content must still be skipped, never resumed.
use crate::models::DocumentStatus;
use archon_cozo::{StoreBusy, with_guarded_failure};
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

const TEXT: &str = "A paused registration finishes on the next run.\n\nSecond paragraph.";

fn pause_once(context: &'static str) -> impl FnMut(&str) -> Option<anyhow::Error> {
    let mut fired = false;
    move |actual| {
        (actual == context && !std::mem::replace(&mut fired, true)).then(|| {
            StoreBusy {
                context: actual.into(),
                attempts: 1,
                detail: "injected store pause".into(),
            }
            .into()
        })
    }
}

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

fn source_dir(temp: &tempfile::TempDir) -> std::path::PathBuf {
    let sources = temp.path().join("sources");
    std::fs::create_dir(&sources).unwrap();
    std::fs::write(sources.join("document.txt"), TEXT).unwrap();
    sources
}

/// Row counts of one clean ingest of the same bytes, for duplicate detection.
fn clean_counts() -> (usize, usize) {
    let temp = tempfile::tempdir().unwrap();
    let db = crate::open_docs_db_for_test(temp.path().join("clean.db")).unwrap();
    ingest(db.db(), &source_dir(&temp)).unwrap();
    let docs = crate::store::list_doc_sources(db.db()).unwrap();
    let id = &docs[0].document_id;
    (
        crate::store::list_pages_for_doc(db.db(), id).unwrap().len(),
        crate::store::list_chunks_for_doc(db.db(), id)
            .unwrap()
            .len(),
    )
}

fn resume_after_pause_at(context: &'static str, paused_status: DocumentStatus) {
    let temp = tempfile::tempdir().unwrap();
    let db = crate::open_docs_db_for_test(temp.path().join("docs.db")).unwrap();
    let sources = source_dir(&temp);
    let paused = with_guarded_failure(pause_once(context), || ingest(db.db(), &sources));
    let error = paused.expect_err("the injected pause must end the run");
    assert!(StoreBusy::find(error.as_ref()).is_some(), "{error:#}");
    let docs = crate::store::list_doc_sources(db.db()).unwrap();
    assert_eq!(docs.len(), 1, "the pause came after registration");
    assert_eq!(docs[0].status, paused_status);

    let resumed = ingest(db.db(), &sources).unwrap();
    assert_eq!(
        (
            resumed.sources_registered,
            resumed.sources_skipped_duplicate
        ),
        (1, 0),
        "the next run must finish the paused document, not skip it"
    );
    assert_eq!(resumed.sources_failed, 0, "{:?}", resumed.errors);
    let docs = crate::store::list_doc_sources(db.db()).unwrap();
    assert_eq!(docs.len(), 1, "resume must reuse the registration");
    assert_eq!(docs[0].status, DocumentStatus::Ingested);
    let id = &docs[0].document_id;
    let counts = (
        crate::store::list_pages_for_doc(db.db(), id).unwrap().len(),
        crate::store::list_chunks_for_doc(db.db(), id)
            .unwrap()
            .len(),
    );
    assert_eq!(counts, clean_counts(), "resume must not keep partial rows");
    assert!(counts.1 > 0);

    let again = ingest(db.db(), &sources).unwrap();
    assert_eq!(
        again.sources_skipped_duplicate, 1,
        "a finished document is a duplicate"
    );
}

#[test]
fn directory_ingest_resumes_a_registration_paused_at_job_creation() {
    resume_after_pause_at("insert doc_processing_jobs", DocumentStatus::Discovered);
}

#[test]
fn directory_ingest_resumes_a_registration_paused_at_page_creation() {
    resume_after_pause_at("insert doc_pages", DocumentStatus::Ingesting);
}

#[test]
fn directory_ingest_resumes_a_registration_paused_at_chunk_creation() {
    resume_after_pause_at("insert doc_chunks", DocumentStatus::Ingesting);
}

#[test]
fn text_source_resumes_a_registration_paused_before_its_artifacts() {
    let temp = tempfile::tempdir().unwrap();
    let db = crate::open_docs_db_for_test(temp.path().join("docs.db")).unwrap();
    let ingest_text =
        || crate::ingest_text::ingest_text_source(db.db(), "https://fixture", "text/plain", TEXT);
    let error = with_guarded_failure(pause_once("insert doc_processing_jobs"), ingest_text)
        .expect_err("the injected pause must end the ingest");
    assert!(
        std::error::Error::source(&error)
            .and_then(|source| source.downcast_ref::<StoreBusy>())
            .is_some(),
        "{error}"
    );
    let resumed = ingest_text().unwrap();
    assert!(resumed.was_new, "the paused registration must be finished");
    assert!(resumed.chunks_registered > 0);
    let docs = crate::store::list_doc_sources(db.db()).unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].status, DocumentStatus::Ingested);
    assert!(!ingest_text().unwrap().was_new);
}

#[test]
fn a_live_ingest_of_the_same_content_is_skipped_not_resumed() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("docs.db");
    let first_db = crate::open_docs_db_for_test(&path).unwrap();
    crate::schema::ensure_doc_schema(first_db.db()).unwrap();
    let sources = source_dir(&temp);
    let (inside, entered) = mpsc::channel();
    let (release, released) = mpsc::channel::<()>();
    let first_sources = sources.clone();
    let first = std::thread::spawn(move || {
        let mut held = false;
        with_guarded_failure(
            move |context| {
                if context == "insert doc_pages" && !std::mem::replace(&mut held, true) {
                    inside.send(()).unwrap();
                    released.recv_timeout(Duration::from_secs(60)).unwrap();
                }
                None
            },
            || ingest(first_db.db(), &first_sources),
        )
    });
    entered.recv_timeout(Duration::from_secs(60)).unwrap();
    let second_db = crate::open_docs_db_for_test(&path).unwrap();
    let concurrent = ingest(second_db.db(), &sources);
    release.send(()).unwrap();
    let first = first.join().unwrap().unwrap();
    let concurrent = concurrent.unwrap();
    assert_eq!(first.sources_registered, 1);
    assert_eq!(
        (
            concurrent.sources_registered,
            concurrent.sources_skipped_duplicate
        ),
        (0, 1),
        "a live ingest owns its registration"
    );
    let docs = crate::store::list_doc_sources(second_db.db()).unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].status, DocumentStatus::Ingested);
    let id = &docs[0].document_id;
    assert_eq!(
        (
            crate::store::list_pages_for_doc(second_db.db(), id)
                .unwrap()
                .len(),
            crate::store::list_chunks_for_doc(second_db.db(), id)
                .unwrap()
                .len(),
        ),
        clean_counts()
    );
}
