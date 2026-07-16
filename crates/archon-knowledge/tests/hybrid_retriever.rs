use archon_docs::models::ChunkArtifact;
use archon_knowledge::hybrid_retriever::{SearchMode, SearchOptions};
use archon_knowledge::{KnowledgeEngine, store};
use cozo::DbInstance;

fn db_with_doc_schema() -> DbInstance {
    let db = DbInstance::new("mem", "", "").unwrap();
    archon_docs::schema::ensure_doc_schema(&db).unwrap();
    db
}

fn engine(db: DbInstance) -> KnowledgeEngine {
    KnowledgeEngine::new(db).unwrap()
}

fn insert_chunk(db: &DbInstance, id: &str, doc: &str, content: &str) {
    archon_docs::store::insert_chunk(
        db,
        &ChunkArtifact {
            chunk_id: id.into(),
            document_id: doc.into(),
            artifact_id: format!("artifact-{id}"),
            chunk_index: 0,
            page_start: 1,
            page_end: 1,
            content: content.into(),
            content_hash: format!("hash-{id}"),
            embedding_status: "pending".into(),
        },
    )
    .unwrap();
}

#[test]
fn exact_search_finds_known_chunk() {
    let db = db_with_doc_schema();
    insert_chunk(
        &db,
        "c1",
        "doc-1",
        "Plugin marketplace incentives reward quality.",
    );
    let engine = engine(db);
    let results = engine
        .search(
            "marketplace quality",
            &SearchOptions {
                mode: SearchMode::Exact,
                top_k: 5,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(results[0].artifact_id, "c1");
}

#[test]
fn exact_search_preserves_any_term_recall() {
    let db = db_with_doc_schema();
    insert_chunk(&db, "c1", "doc-1", "Marketplace incentives.");
    let engine = engine(db);

    let results = engine
        .search(
            "marketplace quality",
            &SearchOptions {
                mode: SearchMode::Exact,
                top_k: 5,
                ..Default::default()
            },
        )
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].artifact_id, "c1");
    assert_eq!(results[0].exact_score, 0.5);
}

#[test]
fn exact_search_preserves_stopword_terms() {
    let db = db_with_doc_schema();
    insert_chunk(&db, "c1", "doc-1", "Research and development.");
    let engine = engine(db);

    let results = engine
        .search(
            "and",
            &SearchOptions {
                mode: SearchMode::Exact,
                top_k: 5,
                ..Default::default()
            },
        )
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].artifact_id, "c1");
    assert_eq!(results[0].exact_score, 1.0);
}

#[test]
fn exact_search_preserves_ascii_boundaries_next_to_unicode() {
    let db = db_with_doc_schema();
    insert_chunk(&db, "c1", "doc-1", "A café guide.");
    let engine = engine(db);

    let results = engine
        .search(
            "caf",
            &SearchOptions {
                mode: SearchMode::Exact,
                top_k: 5,
                ..Default::default()
            },
        )
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].artifact_id, "c1");
    assert_eq!(results[0].exact_score, 1.0);
}

#[test]
fn exact_search_ranks_all_index_matches_by_term_coverage() {
    let db = db_with_doc_schema();
    for index in 0..4 {
        insert_chunk(
            &db,
            &format!("marketplace-{index}"),
            "doc-other",
            "Marketplace.",
        );
        insert_chunk(&db, &format!("quality-{index}"), "doc-other", "Quality.");
    }
    let filler = " filler".repeat(500);
    insert_chunk(
        &db,
        "wanted",
        "doc-wanted",
        &format!("Marketplace quality.{filler}"),
    );
    let engine = engine(db);

    let results = engine
        .search(
            "marketplace quality",
            &SearchOptions {
                mode: SearchMode::Exact,
                top_k: 1,
                ..Default::default()
            },
        )
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].artifact_id, "wanted");
    assert_eq!(results[0].exact_score, 1.0);
}

#[test]
fn exact_search_respects_document_filter() {
    let db = db_with_doc_schema();
    insert_chunk(&db, "c1", "doc-1", "Elliott wave invalidation rules.");
    insert_chunk(&db, "c2", "doc-2", "Elliott wave unrelated archive.");
    let engine = engine(db);
    let results = engine
        .search(
            "Elliott wave",
            &SearchOptions {
                mode: SearchMode::Exact,
                top_k: 5,
                document_filter: Some(vec!["doc-1".into()]),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].document_id, "doc-1");
}

#[test]
fn hybrid_search_uses_exact_when_no_embedding_is_available() {
    let db = db_with_doc_schema();
    insert_chunk(
        &db,
        "c1",
        "doc-1",
        "Strategic workflow evidence is inspectable.",
    );
    let engine = engine(db);
    let results = engine
        .search("workflow evidence", &SearchOptions::default())
        .unwrap();
    assert_eq!(results.len(), 1);
    assert!(results[0].combined_score > 0.0);
}

#[test]
fn semantic_search_without_doc_schema_returns_empty() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let engine = engine(db);

    let results = engine
        .search(
            "semantic",
            &SearchOptions {
                mode: SearchMode::Semantic,
                top_k: 5,
                query_embedding: Some(vec![1.0, 0.0]),
                ..Default::default()
            },
        )
        .unwrap();

    assert!(results.is_empty());
}

#[test]
fn hybrid_search_without_doc_schema_returns_empty() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let engine = engine(db);

    let results = engine
        .search(
            "semantic",
            &SearchOptions {
                mode: SearchMode::Hybrid,
                top_k: 5,
                query_embedding: Some(vec![1.0, 0.0]),
                ..Default::default()
            },
        )
        .unwrap();

    assert!(results.is_empty());
}

#[test]
fn semantic_search_without_vector_schema_returns_empty() {
    let db = db_with_doc_schema();
    insert_chunk(&db, "c1", "doc-1", "A chunk exists without vectors.");
    let engine = engine(db);

    let results = engine
        .search(
            "chunk",
            &SearchOptions {
                mode: SearchMode::Semantic,
                top_k: 5,
                query_embedding: Some(vec![1.0, 0.0]),
                ..Default::default()
            },
        )
        .unwrap();

    assert!(results.is_empty());
}

#[test]
fn semantic_search_without_embedding_returns_empty() {
    let db = db_with_doc_schema();
    insert_chunk(&db, "c1", "doc-1", "A chunk exists.");
    let engine = engine(db);
    let results = engine
        .search(
            "chunk",
            &SearchOptions {
                mode: SearchMode::Semantic,
                top_k: 5,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(results.is_empty());
}

#[test]
fn semantic_search_rejects_top_k_above_cozo_integer_range() {
    let db = db_with_doc_schema();
    archon_docs::schema::ensure_vec_schema(&db, 2).unwrap();
    insert_chunk(&db, "c1", "doc-1", "Semantic target chunk.");
    archon_docs::store::insert_chunk_embedding(&db, "c1", &[1.0, 0.0], "test").unwrap();
    let engine = engine(db);

    let error = engine
        .search(
            "semantic target",
            &SearchOptions {
                mode: SearchMode::Semantic,
                top_k: usize::MAX,
                query_embedding: Some(vec![1.0, 0.0]),
                ..Default::default()
            },
        )
        .unwrap_err();

    assert_eq!(
        error.to_string(),
        "invalid search options: top_k exceeds CozoDB's signed integer range"
    );
}

#[test]
fn semantic_search_finds_vector_indexed_chunk() {
    let db = db_with_doc_schema();
    archon_docs::schema::ensure_vec_schema(&db, 2).unwrap();
    insert_chunk(&db, "c1", "doc-1", "Semantic target chunk.");
    insert_chunk(&db, "c2", "doc-2", "Different vector chunk.");
    archon_docs::store::insert_chunk_embedding(&db, "c1", &[1.0, 0.0], "test").unwrap();
    archon_docs::store::insert_chunk_embedding(&db, "c2", &[0.0, 1.0], "test").unwrap();
    let engine = engine(db);
    let results = engine
        .search(
            "semantic target",
            &SearchOptions {
                mode: SearchMode::Semantic,
                top_k: 1,
                query_embedding: Some(vec![1.0, 0.0]),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(results[0].artifact_id, "c1");
    assert!(results[0].semantic_score > 0.9);
}

#[test]
fn semantic_search_filter_overfetches_before_post_filtering() {
    let db = db_with_doc_schema();
    archon_docs::schema::ensure_vec_schema(&db, 2).unwrap();
    for index in 0..3 {
        let chunk_id = format!("excluded-{index}");
        insert_chunk(&db, &chunk_id, "doc-other", "Closer excluded vector.");
        archon_docs::store::insert_chunk_embedding(&db, &chunk_id, &[1.0, 0.0], "test").unwrap();
    }
    insert_chunk(&db, "wanted", "doc-wanted", "Allowed vector.");
    archon_docs::store::insert_chunk_embedding(&db, "wanted", &[0.0, 1.0], "test").unwrap();
    let engine = engine(db);

    let results = engine
        .search(
            "vector",
            &SearchOptions {
                mode: SearchMode::Semantic,
                top_k: 1,
                query_embedding: Some(vec![1.0, 0.0]),
                document_filter: Some(vec!["doc-wanted".into()]),
                ..Default::default()
            },
        )
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].artifact_id, "wanted");
}

#[test]
fn semantic_search_respects_document_filter() {
    let db = db_with_doc_schema();
    archon_docs::schema::ensure_vec_schema(&db, 2).unwrap();
    insert_chunk(&db, "c1", "doc-1", "Filtered vector chunk.");
    insert_chunk(&db, "c2", "doc-2", "Closer global vector chunk.");
    archon_docs::store::insert_chunk_embedding(&db, "c1", &[0.8, 0.2], "test").unwrap();
    archon_docs::store::insert_chunk_embedding(&db, "c2", &[1.0, 0.0], "test").unwrap();
    let engine = engine(db);
    let results = engine
        .search(
            "vector",
            &SearchOptions {
                mode: SearchMode::Semantic,
                top_k: 1,
                query_embedding: Some(vec![1.0, 0.0]),
                document_filter: Some(vec!["doc-1".into()]),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].document_id, "doc-1");
}

#[test]
fn list_doc_chunks_missing_doc_schema_returns_empty() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let rows = store::list_doc_chunks(&db).unwrap();
    assert!(rows.is_empty());
}

#[test]
fn fts_chunk_candidates_reject_limit_above_cozo_integer_range() {
    let db = db_with_doc_schema();

    let error = store::search_doc_chunks_fts(&db, "marketplace", usize::MAX, None).unwrap_err();

    assert_eq!(
        error.to_string(),
        "invalid search options: limit exceeds CozoDB's signed integer range"
    );
}

#[test]
fn fts_chunk_candidates_use_the_docs_index() {
    let db = db_with_doc_schema();
    insert_chunk(
        &db,
        "c1",
        "doc-1",
        "Plugin marketplace incentives reward quality.",
    );
    insert_chunk(&db, "c2", "doc-2", "Unrelated archive text.");

    let rows = store::search_doc_chunks_fts(&db, "marketplace quality", 5, None).unwrap();

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].chunk_id, "c1");
}

#[test]
fn fts_chunk_candidates_respect_document_filter() {
    let db = db_with_doc_schema();
    insert_chunk(&db, "c1", "doc-1", "Plugin marketplace quality.");
    insert_chunk(&db, "c2", "doc-2", "Plugin marketplace quality.");
    let filter = vec!["doc-2".to_string()];

    let rows = store::search_doc_chunks_fts(&db, "marketplace quality", 5, Some(&filter)).unwrap();

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].chunk_id, "c2");
}

#[test]
fn semantic_search_with_empty_document_filter_returns_empty() {
    let db = db_with_doc_schema();
    archon_docs::schema::ensure_vec_schema(&db, 2).unwrap();
    insert_chunk(&db, "c1", "doc-1", "Semantic target chunk.");
    archon_docs::store::insert_chunk_embedding(&db, "c1", &[1.0, 0.0], "test").unwrap();
    let engine = engine(db);

    let results = engine
        .search(
            "semantic target",
            &SearchOptions {
                mode: SearchMode::Semantic,
                top_k: 1,
                query_embedding: Some(vec![1.0]),
                document_filter: Some(Vec::new()),
                ..Default::default()
            },
        )
        .unwrap();

    assert!(results.is_empty());
}

#[test]
fn filtered_fts_applies_filter_before_limit() {
    let db = db_with_doc_schema();
    for index in 0..5 {
        insert_chunk(
            &db,
            &format!("other-{index}"),
            "doc-other",
            "Marketplace quality quality quality.",
        );
    }
    insert_chunk(&db, "wanted", "doc-wanted", "Marketplace quality.");
    let filter = vec!["doc-wanted".to_string()];

    let rows = store::search_doc_chunks_fts(&db, "marketplace quality", 1, Some(&filter)).unwrap();

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].chunk_id, "wanted");
}

#[test]
fn exact_search_without_doc_schema_returns_empty() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let engine = engine(db);

    let results = engine
        .search(
            "marketplace",
            &SearchOptions {
                mode: SearchMode::Exact,
                top_k: 5,
                ..Default::default()
            },
        )
        .unwrap();

    assert!(results.is_empty());
}

#[test]
fn doc_chunk_count_does_not_require_materializing_chunks() {
    let db = db_with_doc_schema();
    insert_chunk(&db, "c1", "doc-1", "First chunk.");
    insert_chunk(&db, "c2", "doc-2", "Second chunk.");

    assert_eq!(store::count_doc_chunks(&db).unwrap(), 2);
}
