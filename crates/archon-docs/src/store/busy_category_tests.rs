use archon_cozo::{StoreBusy, with_guarded_failure};

fn inject<T>(context: &'static str, run: impl FnOnce() -> T) -> T {
    with_guarded_failure(
        move |c| {
            (c == context).then(|| {
                StoreBusy {
                    context: c.into(),
                    attempts: 1,
                    detail: "writer acquisition paused".into(),
                }
                .into()
            })
        },
        run,
    )
}
#[test]
fn interrupted_chunk_list_retains_busy_category() {
    let db = cozo::DbInstance::new("mem", "", "").unwrap();
    let error = inject("list chunks for doc", || {
        super::list_chunks_for_doc(&db, "doc")
    })
    .unwrap_err();
    assert!(error.is::<StoreBusy>(), "busy must stay typed: {error}");
}
#[test]
fn interrupted_text_ingest_retains_busy_category() {
    let db = cozo::DbInstance::new("mem", "", "").unwrap();
    let error = inject("schema creation", || {
        crate::ingest_text::ingest_text_source(&db, "fixture", "text/plain", "text")
    })
    .unwrap_err();
    assert!(
        std::error::Error::source(&error)
            .and_then(|source| source.downcast_ref::<StoreBusy>())
            .is_some(),
        "busy must stay typed: {error}"
    );
}
#[test]
fn directory_pauses_on_busy_instead_of_counting_a_failed_source() {
    let db = cozo::DbInstance::new("mem", "", "").unwrap();
    crate::schema::ensure_doc_schema(&db).unwrap();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("source.txt"), "content").unwrap();
    let error = with_guarded_failure(
        |c| {
            (c == "insert doc_processing_jobs").then(|| {
                StoreBusy {
                    context: c.into(),
                    attempts: 1,
                    detail: "writer acquisition paused".into(),
                }
                .into()
            })
        },
        || {
            // poll on this thread so the thread-local statement seam remains scoped
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(crate::ingest_directory::ingest_directory(&db, dir.path()))
        },
    );
    let error = error.expect_err("a busy source must pause, never yield a completed tally");
    assert!(
        error.is::<StoreBusy>(),
        "directory callers must receive a typed pause: {error}"
    );
}

fn integrity_pause(context: &'static str) {
    let db = cozo::DbInstance::new("mem", "", "").unwrap();
    crate::schema::ensure_doc_schema(&db).unwrap();
    let chunk = crate::models::ChunkArtifact {
        chunk_id: "chunk".into(),
        document_id: "doc".into(),
        artifact_id: "artifact".into(),
        chunk_index: 0,
        page_start: 1,
        page_end: 1,
        content: "text".into(),
        content_hash: "hash".into(),
        embedding_status: "pending".into(),
    };
    let error = inject(context, || {
        crate::provenance_chunks::persist_chunk_integrity(
            &db,
            "artifact",
            &[chunk],
            &Default::default(),
            "engine",
            "hash",
            "run",
        )
    })
    .unwrap_err();
    assert!(
        std::error::Error::source(&error)
            .and_then(|s| s.downcast_ref::<StoreBusy>())
            .is_some(),
        "nested ingest pause must stay typed: {error}"
    );
}
#[test]
fn chunk_hash_ingest_pause_remains_typed() {
    integrity_pause("insert doc_chunk_hashes");
}
#[test]
fn nested_provenance_schema_pause_remains_typed() {
    integrity_pause("provenance schema: create relation");
}
#[test]
fn nested_provenance_write_pause_remains_typed() {
    integrity_pause("provenance store: insert prov_records row");
}

#[test]
fn provenance_read_pause_is_not_an_absent_record() {
    let db = cozo::DbInstance::new("mem", "", "").unwrap();
    let error = inject("provenance store: get record", || {
        crate::provenance_chunks::verify_chunks_root(&db, "doc", "record")
    })
    .expect_err("an unsuccessful provenance read must not become an absent record");
    assert!(
        std::error::Error::source(&error)
            .and_then(|s| s.downcast_ref::<StoreBusy>())
            .is_some(),
        "read pause must stay typed: {error}"
    );
}
