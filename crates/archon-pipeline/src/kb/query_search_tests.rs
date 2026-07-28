use std::sync::Arc;

use archon_docs::embed::LocalEmbeddingProvider;

use super::*;
use crate::kb::schema::{ensure_kb_embedding_schema, ensure_kb_schema};

struct FilteredEmbeddingProvider;

impl LocalEmbeddingProvider for FilteredEmbeddingProvider {
    fn embed_chunks(
        &self,
        chunks: &[String],
    ) -> Result<Vec<Vec<f32>>, archon_docs::errors::DocsError> {
        Ok(chunks.iter().map(|chunk| vector_for(chunk)).collect())
    }

    fn embed_query(&self, query: &str) -> Result<Vec<f32>, archon_docs::errors::DocsError> {
        Ok(vector_for(query))
    }

    fn dimension(&self) -> usize {
        2
    }

    fn backend_name(&self) -> &'static str {
        "filtered-test"
    }
}

fn vector_for(text: &str) -> Vec<f32> {
    if text.contains("concept-match") {
        vec![0.8, 0.6]
    } else {
        vec![1.0, 0.0]
    }
}

fn semantic_test_db() -> cozo::DbInstance {
    let db = cozo::DbInstance::new("mem", "", Default::default()).unwrap();
    ensure_kb_schema(&db).unwrap();
    ensure_kb_embedding_schema(&db, "filtered-test", 2, None).unwrap();
    db
}

#[test]
fn semantic_search_applies_type_filter_before_limiting_candidates() {
    let db = semantic_test_db();
    for index in 0..4 {
        insert_node(
            &db,
            &format!("raw-{index}"),
            "raw",
            &format!("raw-match-{index}"),
        );
    }
    insert_node(&db, "concept", "concept", "concept-match");
    let engine = QueryEngine::new(db).with_embedder(Arc::new(FilteredEmbeddingProvider));

    let results = engine
        .search_nodes("vehicle", 1, Some(&[KbNodeType::Concept]))
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].node.node_id, "concept");
}

#[test]
fn search_rejects_limits_that_exceed_cozo_integer_range() {
    let db = semantic_test_db();
    let engine = QueryEngine::new(db);

    let error = engine
        .search_nodes("vehicle", usize::MAX, None)
        .unwrap_err();

    assert!(error.to_string().contains("KB search limit is too large"));
}

#[test]
fn semantic_search_penalizes_answer_nodes_after_merging() {
    let db = semantic_test_db();
    insert_node(&db, "raw", "raw", "raw-match");
    insert_node(&db, "answer", "answer", "answer-match");
    let engine = QueryEngine::new(db).with_embedder(Arc::new(FilteredEmbeddingProvider));

    let results = engine.search_nodes("vehicle", 2, None).unwrap();

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].node.node_id, "raw");
    assert_eq!(results[1].node.node_id, "answer");
    assert!((results[1].score - results[0].score * 0.9).abs() < 0.01);
}

fn insert_node(db: &cozo::DbInstance, id: &str, node_type: &str, content: &str) {
    let mut params = BTreeMap::new();
    params.insert("node_id".into(), DataValue::from(id));
    params.insert("node_type".into(), DataValue::from(node_type));
    params.insert("content".into(), DataValue::from(content));
    db.run_script(
        "?[node_id, node_type, source, domain_tag, title, content, content_hash, \
         chunk_index, created_at, updated_at] <- [[$node_id, $node_type, 'test', '', \
         '', $content, '', 0, 1.0, 1.0]] \
         :put kb_nodes { node_id => node_type, source, domain_tag, title, content, \
         content_hash, chunk_index, created_at, updated_at }",
        params,
        ScriptMutability::Mutable,
    )
    .unwrap();
}
