use super::*;

fn fixture() -> cozo::DbInstance {
    let db = cozo::DbInstance::new("mem", "", "").unwrap();
    crate::schema::ensure_doc_schema(&db).unwrap();
    store::insert_page(
        &db,
        &crate::models::PageArtifact {
            page_id: "page".into(),
            document_id: "doc".into(),
            page_number: 7,
            text_hash: None,
            image_hash: None,
            width: None,
            height: None,
            provenance_record_id: "prov".into(),
        },
    )
    .unwrap();
    db
}
fn hits() -> cozo::NamedRows {
    cozo::NamedRows {
        headers: vec!["page_id".into(), "distance".into()],
        rows: vec![vec![DataValue::from("page-img2"), DataValue::from(0.2)]],
        next: None,
    }
}
fn busy_at(context: &'static str) {
    let db = fixture();
    // These rows represent a successful vector query before the peer writes.
    let error = archon_cozo::with_guarded_failure(
        move |c| {
            (c == context).then(|| {
                archon_cozo::StoreBusy {
                    context: c.into(),
                    attempts: 1,
                    detail: "peer still holds a write transaction".into(),
                }
                .into()
            })
        },
        || resolve_hits(&db, hits()),
    )
    .expect_err("an unsuccessful provenance read must not produce successful hits");
    assert!(
        std::error::Error::source(&error)
            .and_then(|source| source.downcast_ref::<archon_cozo::StoreBusy>())
            .is_some(),
        "busy must retain its typed source: {error}"
    );
}
#[test]
fn exhausted_page_read_after_vector_query_is_not_fabricated_provenance() {
    busy_at("resolve page");
}
#[test]
fn exhausted_source_read_after_vector_query_is_not_empty_source() {
    busy_at("get doc source");
}
#[test]
fn failed_page_read_is_distinct_from_an_absent_page() {
    let db = fixture();
    let result = archon_cozo::with_guarded_failure(
        |c| (c == "resolve page").then(|| anyhow::anyhow!("invalid page read")),
        || resolve_hits(&db, hits()),
    );
    assert!(result.is_err(), "read failure must propagate");
}

fn persisted_hits(db: &DbInstance) -> cozo::NamedRows {
    crate::schema::ensure_doc_schema(db).unwrap();
    crate::schema::ensure_vec_schema(db, 4, Some(4)).unwrap();
    store::insert_doc_source(
        db,
        &crate::models::SourceDocument {
            document_id: "doc".into(),
            source_path: "/image.pdf".into(),
            media_type: "application/pdf".into(),
            content_hash: "hash".into(),
            discovered_at: "now".into(),
            status: crate::models::DocumentStatus::Ingested,
        },
    )
    .unwrap();
    store::insert_page(
        db,
        &crate::models::PageArtifact {
            page_id: "page".into(),
            document_id: "doc".into(),
            page_number: 7,
            text_hash: None,
            image_hash: None,
            width: None,
            height: None,
            provenance_record_id: "prov".into(),
        },
    )
    .unwrap();
    store::insert_page_image_embedding(db, "page-img2", &[1.0, 0.0, 0.0, 0.0], "fixture").unwrap();
    db.run_script("?[page_id, distance] := ~vec_page_images:page_image_embedding_idx{page_id | query: vec([1.0, 0.0, 0.0, 0.0]), k: 1, ef: 50, bind_distance: distance}", Default::default(), ScriptMutability::Immutable).unwrap()
}
fn long_write_after_vector_query(at_source: bool) {
    use std::{cell::Cell, rc::Rc};
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("images.db");
    let db = crate::open_docs_db_for_test(&path).unwrap();
    let rows = persisted_hits(&db);
    assert_eq!(rows.rows.len(), 1, "vector query really completed");
    let connection = Rc::new(sqlite::Connection::open(&path).unwrap());
    let started = Rc::new(Cell::new(false));
    if !at_source {
        connection
            .execute("BEGIN EXCLUSIVE; UPDATE cozo SET v = v;")
            .unwrap();
        started.set(true);
    }
    let stage_connection = connection.clone();
    let stage_started = started.clone();
    let advance = connection.clone();
    let count = Rc::new(Cell::new(0));
    let seen = count.clone();
    let result = archon_cozo::with_guarded_failure(
        move |context| {
            if at_source && context == "get doc source" && !stage_started.replace(true) {
                stage_connection
                    .execute("BEGIN EXCLUSIVE; UPDATE cozo SET v = v;")
                    .unwrap();
            }
            None
        },
        || {
            archon_cozo::with_busy_observer(
                move |_, _| {
                    seen.set(seen.get() + 1);
                    if seen.get() == 24 {
                        advance.execute("COMMIT;").unwrap();
                    } else {
                        advance.execute("UPDATE cozo SET v = v;").unwrap();
                    }
                },
                || resolve_hits(&db, rows),
            )
        },
    );
    // Release even when old code gives up before the writer finishes.
    let _ = connection.execute("COMMIT;");
    let hit = result.unwrap().remove(0);
    assert_eq!(
        count.get(),
        24,
        "must recover the interrupted statement beyond twenty attempts"
    );
    assert_eq!(
        (
            hit.document_id.as_str(),
            hit.page_number,
            hit.source_path.as_str()
        ),
        ("doc", 7, "/image.pdf")
    );
}
#[test]
fn long_write_after_vector_query_keeps_page_provenance() {
    long_write_after_vector_query(false);
}
#[test]
fn long_write_after_page_query_keeps_source_provenance() {
    long_write_after_vector_query(true);
}
#[test]
fn absent_page_is_distinct_from_a_successful_hit_with_defaults() {
    let db = cozo::DbInstance::new("mem", "", "").unwrap();
    crate::schema::ensure_doc_schema(&db).unwrap();
    assert!(
        resolve_hits(&db, hits()).is_err(),
        "dangling image hits must not fabricate provenance"
    );
}
