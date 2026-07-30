use archon_docs::models::ChunkArtifact;
use archon_knowledge::KnowledgeEngine;
use archon_knowledge::hybrid_retriever::{SearchMode, SearchOptions};
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
fn semantic_search_rejects_top_k_above_cozo_integer_range() {
    let db = db_with_doc_schema();
    archon_docs::schema::ensure_vec_schema(&db, 2, None).unwrap();
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
    archon_docs::schema::ensure_vec_schema(&db, 2, None).unwrap();
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
    archon_docs::schema::ensure_vec_schema(&db, 2, None).unwrap();
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
    archon_docs::schema::ensure_vec_schema(&db, 2, None).unwrap();
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
fn semantic_search_with_empty_document_filter_returns_empty() {
    let db = db_with_doc_schema();
    archon_docs::schema::ensure_vec_schema(&db, 2, None).unwrap();
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
