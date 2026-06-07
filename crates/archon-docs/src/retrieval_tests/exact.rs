use super::*;
use crate::retrieval::{RetrievalWeights, SearchMode, search, search_with_mode};
use crate::retrieval_exact::list_chunks_for_exact_fallback;

#[test]
#[serial_test::serial(docs_global_state)]
fn search_empty_corpus() {
    let db = test_db();
    setup_with_provider(&db, 4);

    let results = search(&db, "test query", 5).unwrap();
    assert!(results.results.is_empty());
    assert_eq!(results.total_indexed_chunks, 0);
}

#[test]
#[serial_test::serial(docs_global_state)]
fn exact_search_finds_quoted_string() {
    let db = test_db();
    crate::schema::ensure_doc_schema(&db).unwrap();
    insert_test_chunk(
        &db,
        "chunk-quoted-hit",
        "The audit contains the exact phrase blue falcon protocol.",
    );
    insert_test_chunk(
        &db,
        "chunk-quoted-miss",
        "The audit contains unrelated policy text.",
    );

    let results = search_with_mode(
        &db,
        "\"blue falcon protocol\"",
        5,
        SearchMode::Exact,
        RetrievalWeights::default(),
    )
    .unwrap();

    assert_eq!(results.mode, SearchMode::Exact);
    assert!(!results.results.is_empty());
    assert_eq!(results.results[0].chunk_id, "chunk-quoted-hit");
    assert!(results.results[0].exact_score > 0.0);
    assert_eq!(results.results[0].semantic_score, 0.0);
}

#[test]
#[serial_test::serial(docs_global_state)]
fn exact_no_hit_reports_persisted_chunk_count() {
    let db = test_db();
    crate::schema::ensure_doc_schema(&db).unwrap();
    insert_test_chunk(
        &db,
        "chunk-existing",
        "Stored evidence about unrelated incentives.",
    );

    let results = search_with_mode(
        &db,
        "\"missing phrase\"",
        5,
        SearchMode::Exact,
        RetrievalWeights::default(),
    )
    .unwrap();

    assert!(results.results.is_empty());
    assert_eq!(results.total_chunks, 1);
    assert_eq!(results.total_indexed_chunks, 0);
}

#[test]
#[serial_test::serial(docs_global_state)]
fn exact_scan_fallback_is_bounded() {
    let db = test_db();
    crate::schema::ensure_doc_schema(&db).unwrap();
    for index in 0..5 {
        insert_test_chunk(
            &db,
            &format!("chunk-bounded-{index}"),
            &format!("bounded fallback fixture {index}"),
        );
    }

    let rows = list_chunks_for_exact_fallback(&db, 2).unwrap();
    assert_eq!(rows.len(), 2);

    let no_rows = list_chunks_for_exact_fallback(&db, 0).unwrap();
    assert!(no_rows.is_empty());
}

#[test]
#[serial_test::serial(docs_global_state)]
fn question_punctuation_uses_safe_fts_query() {
    let db = test_db();
    crate::schema::ensure_doc_schema(&db).unwrap();
    insert_test_chunk(
        &db,
        "chunk-traders-reality",
        "Traders Reality Hybrid System explains market maker behaviour.",
    );

    let results = search_with_mode(
        &db,
        "What is the Traders Reality Hybrid System?",
        5,
        SearchMode::Exact,
        RetrievalWeights::default(),
    )
    .unwrap();

    assert_eq!(results.results[0].chunk_id, "chunk-traders-reality");
}
