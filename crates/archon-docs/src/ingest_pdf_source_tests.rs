use super::*;
fn failed_read(busy: bool) -> DocsError {
    let db = DbInstance::new("mem", "", "").unwrap();
    crate::schema::ensure_doc_schema(&db).unwrap();
    archon_cozo::with_guarded_failure(
        move |context| {
            (context == "get doc source").then(|| {
                if busy {
                    archon_cozo::StoreBusy {
                        context: context.into(),
                        attempts: 1,
                        detail: "paused".into(),
                    }
                    .into()
                } else {
                    anyhow::anyhow!("unsuccessful source read")
                }
            })
        },
        || source_hash(&db, "doc"),
    )
    .expect_err("an unsuccessful read must not fabricate an empty input hash")
}
#[test]
fn pdf_source_pause_stays_typed() {
    assert!(matches!(failed_read(true), DocsError::StoreBusy(_)));
}
#[test]
fn pdf_source_read_failure_is_not_an_empty_hash() {
    assert!(matches!(failed_read(false), DocsError::Storage { .. }));
}
#[test]
fn missing_pdf_source_is_not_an_empty_hash() {
    let db = DbInstance::new("mem", "", "").unwrap();
    crate::schema::ensure_doc_schema(&db).unwrap();
    assert!(source_hash(&db, "missing").is_err());
}
