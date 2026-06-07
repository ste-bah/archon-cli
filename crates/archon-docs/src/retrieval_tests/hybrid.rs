use super::*;
use crate::retrieval::{RetrievalWeights, SearchMode, index_chunk, search_with_mode};

#[test]
#[serial_test::serial(docs_global_state)]
fn hybrid_skips_semantic_when_exact_evidence_is_strong() {
    let db = test_db();
    crate::schema::ensure_doc_schema(&db).unwrap();
    for index in 0..3 {
        insert_test_chunk(
            &db,
            &format!("chunk-hybrid-system-{index}"),
            "Traders Reality Hybrid System course component methodology.",
        );
    }

    let results = search_with_mode(
        &db,
        "What is the Traders Reality Hybrid System?",
        3,
        SearchMode::Hybrid,
        RetrievalWeights::default(),
    )
    .unwrap();

    assert_eq!(results.results.len(), 3);
    assert_eq!(results.query_embedding_norm, None);
    assert!(
        results
            .warnings
            .iter()
            .any(|warning| warning.contains("high-confidence lexical"))
    );
}

#[test]
#[serial_test::serial(docs_global_state)]
fn definition_exact_hit_skips_semantic_embedding() {
    let db = test_db();
    crate::schema::ensure_doc_schema(&db).unwrap();
    crate::schema::ensure_vec_schema(&db, 4).unwrap();
    crate::embed::set_provider(Box::new(SynonymProvider));
    let chunk = insert_test_chunk(
        &db,
        "chunk-traders-hybrid",
        "The Traders Reality Hybrid System explains market maker behaviour.",
    );
    index_chunk(&db, &chunk).unwrap();

    crate::embed::set_provider(Box::new(FailingQueryProvider));
    let results = search_with_mode(
        &db,
        "What do you know about the Traders Reality Hybrid System?",
        3,
        SearchMode::Hybrid,
        RetrievalWeights::default(),
    )
    .unwrap();

    assert_eq!(results.results[0].chunk_id, "chunk-traders-hybrid");
    assert_eq!(results.query_embedding_norm, None);
    assert!(
        results
            .warnings
            .iter()
            .any(|warning| warning.contains("high-confidence lexical"))
    );
}

#[test]
#[serial_test::serial(docs_global_state)]
fn definition_typo_uses_relaxed_exact_before_semantic() {
    let db = test_db();
    crate::schema::ensure_doc_schema(&db).unwrap();
    crate::schema::ensure_vec_schema(&db, 4).unwrap();
    crate::embed::set_provider(Box::new(SynonymProvider));
    let chunk = insert_test_chunk(
        &db,
        "chunk-typo-hybrid",
        "The Traders Reality Hybrid System course explains market maker behaviour.",
    );
    index_chunk(&db, &chunk).unwrap();

    crate::embed::set_provider(Box::new(FailingQueryProvider));
    let results = search_with_mode(
        &db,
        "what do you know about the hybrid systm",
        3,
        SearchMode::Hybrid,
        RetrievalWeights::default(),
    )
    .unwrap();

    assert_eq!(results.results[0].chunk_id, "chunk-typo-hybrid");
    assert_eq!(results.query_embedding_norm, None);
}

#[test]
#[serial_test::serial(docs_global_state)]
fn semantic_search_finds_synonym() {
    let db = test_db();
    crate::schema::ensure_doc_schema(&db).unwrap();
    crate::schema::ensure_vec_schema(&db, 4).unwrap();
    crate::embed::set_provider(Box::new(SynonymProvider));
    let car = insert_test_chunk(
        &db,
        "chunk-car",
        "A car accelerates quickly through the junction.",
    );
    let fruit = insert_test_chunk(&db, "chunk-fruit", "Bananas ripen in warm storage rooms.");
    index_chunk(&db, &car).unwrap();
    index_chunk(&db, &fruit).unwrap();

    let results = search_with_mode(
        &db,
        "automobile",
        2,
        SearchMode::Semantic,
        RetrievalWeights::default(),
    )
    .unwrap();

    assert_eq!(results.mode, SearchMode::Semantic);
    assert_eq!(results.results[0].chunk_id, "chunk-car");
    assert!(results.results[0].semantic_score > 0.9);
    assert!(results.query_embedding_norm.unwrap_or(0.0) > 0.0);
}

#[test]
#[serial_test::serial(docs_global_state)]
fn hybrid_outperforms_either_alone_on_fixture() {
    let db = test_db();
    crate::schema::ensure_doc_schema(&db).unwrap();
    crate::schema::ensure_vec_schema(&db, 4).unwrap();
    crate::embed::set_provider(Box::new(SynonymProvider));
    let exact_only = insert_test_chunk(
        &db,
        "chunk-exact",
        "EXACT_ONLY market signal market signal unrelated ledger note.",
    );
    let semantic_only = insert_test_chunk(
        &db,
        "chunk-semantic",
        "PURE_SEMANTIC vehicle coalition dynamics with no query terms.",
    );
    let hybrid_target = insert_test_chunk(
        &db,
        "chunk-hybrid",
        "HYBRID_TARGET market cooperative transport alignment.",
    );
    index_chunk(&db, &exact_only).unwrap();
    index_chunk(&db, &semantic_only).unwrap();
    index_chunk(&db, &hybrid_target).unwrap();

    let weights = RetrievalWeights {
        exact: 0.5,
        semantic: 0.5,
    };
    let exact = search_with_mode(&db, "market signal", 3, SearchMode::Exact, weights).unwrap();
    let semantic =
        search_with_mode(&db, "market signal", 3, SearchMode::Semantic, weights).unwrap();
    let hybrid = search_with_mode(&db, "market signal", 3, SearchMode::Hybrid, weights).unwrap();

    assert_eq!(exact.results[0].chunk_id, "chunk-exact");
    assert_eq!(semantic.results[0].chunk_id, "chunk-semantic");
    assert_eq!(hybrid.results[0].chunk_id, "chunk-hybrid");
    assert!(hybrid.query_embedding_norm.unwrap_or(0.0) > 0.0);
    assert!(hybrid.results[0].exact_score > 0.0);
    assert!(hybrid.results[0].semantic_score > 0.0);
    assert!(hybrid.results[0].score > hybrid.results[1].score);
}

#[test]
#[serial_test::serial(docs_global_state)]
fn hybrid_propagates_real_embedding_errors() {
    let db = test_db();
    crate::schema::ensure_doc_schema(&db).unwrap();
    crate::schema::ensure_vec_schema(&db, 4).unwrap();
    crate::embed::set_provider(Box::new(SynonymProvider));
    let chunk = insert_test_chunk(&db, "chunk-vectorized", "A car market fixture.");
    index_chunk(&db, &chunk).unwrap();
    assert_eq!(store::count_indexed_chunks(&db).unwrap(), 1);

    crate::embed::set_provider(Box::new(FailingQueryProvider));
    let err = search_with_mode(
        &db,
        "automobile",
        5,
        SearchMode::Hybrid,
        RetrievalWeights::default(),
    )
    .unwrap_err();
    assert!(
        err.to_string()
            .contains("synthetic query embedding failure")
    );
}
