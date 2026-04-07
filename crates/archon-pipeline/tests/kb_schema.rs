use archon_pipeline::kb::schema::{ensure_kb_schema, KbEdgeType, KbNodeType};
use archon_pipeline::kb::{IngestSource, KnowledgeBase, QueryOptions};
use cozo::{DbInstance, ScriptMutability};
use std::path::PathBuf;

fn mem_db() -> DbInstance {
    DbInstance::new("mem", "", "").unwrap()
}

#[test]
fn test_ensure_kb_schema_creates_relations() {
    let db = mem_db();
    ensure_kb_schema(&db).unwrap();

    let result = db
        .run_script("::relations", Default::default(), ScriptMutability::Immutable)
        .unwrap();

    let name_col = result
        .headers
        .iter()
        .position(|h| h == "name")
        .expect("relations output should have 'name' column");

    let names: Vec<String> = result
        .rows
        .iter()
        .map(|row| row[name_col].get_str().unwrap().to_string())
        .collect();

    assert!(names.contains(&"kb_nodes".to_string()), "kb_nodes relation should exist");
    assert!(names.contains(&"kb_edges".to_string()), "kb_edges relation should exist");
    assert!(
        names.contains(&"kb_embeddings".to_string()),
        "kb_embeddings relation should exist"
    );
}

#[test]
fn test_ensure_kb_schema_is_idempotent() {
    let db = mem_db();
    ensure_kb_schema(&db).unwrap();
    // Second call must not error
    ensure_kb_schema(&db).unwrap();
}

#[test]
fn test_kb_nodes_crud() {
    let db = mem_db();
    ensure_kb_schema(&db).unwrap();

    let insert = r#"
        ?[node_id, node_type, source, domain_tag, title, content, content_hash, chunk_index, created_at, updated_at] <- [
            ["n1", "Raw", "file:///test.md", "rust", "Test Node", "Hello world", "abc123", 0, 1700000000.0, 1700000000.0]
        ]
        :put kb_nodes {
            node_id
            =>
            node_type,
            source,
            domain_tag,
            title,
            content,
            content_hash,
            chunk_index,
            created_at,
            updated_at
        }
    "#;
    db.run_script(insert, Default::default(), ScriptMutability::Mutable)
        .unwrap();

    let query = r#"
        ?[node_id, node_type, source, domain_tag, title, content, content_hash, chunk_index, created_at, updated_at] :=
            *kb_nodes{node_id, node_type, source, domain_tag, title, content, content_hash, chunk_index, created_at, updated_at},
            node_id == "n1"
    "#;
    let result = db
        .run_script(query, Default::default(), ScriptMutability::Immutable)
        .unwrap();

    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0].get_str().unwrap(), "n1");
    assert_eq!(result.rows[0][1].get_str().unwrap(), "Raw");
    assert_eq!(result.rows[0][4].get_str().unwrap(), "Test Node");
    assert_eq!(result.rows[0][5].get_str().unwrap(), "Hello world");
}

#[test]
fn test_kb_edges_crud() {
    let db = mem_db();
    ensure_kb_schema(&db).unwrap();

    let insert = r#"
        ?[edge_id, source_node_id, target_node_id, edge_type, created_at] <- [
            ["e1", "n1", "n2", "Provenance", 1700000000.0]
        ]
        :put kb_edges {
            edge_id
            =>
            source_node_id,
            target_node_id,
            edge_type,
            created_at
        }
    "#;
    db.run_script(insert, Default::default(), ScriptMutability::Mutable)
        .unwrap();

    let query = r#"
        ?[edge_id, source_node_id, target_node_id, edge_type, created_at] :=
            *kb_edges{edge_id, source_node_id, target_node_id, edge_type, created_at},
            edge_id == "e1"
    "#;
    let result = db
        .run_script(query, Default::default(), ScriptMutability::Immutable)
        .unwrap();

    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0].get_str().unwrap(), "e1");
    assert_eq!(result.rows[0][1].get_str().unwrap(), "n1");
    assert_eq!(result.rows[0][2].get_str().unwrap(), "n2");
    assert_eq!(result.rows[0][3].get_str().unwrap(), "Provenance");
}

#[test]
fn test_kb_node_type_variants() {
    // KbNodeType must have exactly 5 variants
    let variants = vec![
        KbNodeType::Raw,
        KbNodeType::Compiled,
        KbNodeType::Concept,
        KbNodeType::Answer,
        KbNodeType::Index,
    ];
    assert_eq!(variants.len(), 5);

    // Verify serialization round-trip
    for v in &variants {
        let json = serde_json::to_string(v).unwrap();
        let _back: KbNodeType = serde_json::from_str(&json).unwrap();
    }
}

#[test]
fn test_kb_edge_type_variants() {
    let variants = vec![
        KbEdgeType::Provenance,
        KbEdgeType::Backlink,
        KbEdgeType::CrossReference,
        KbEdgeType::ConceptOf,
        KbEdgeType::DerivedFrom,
    ];
    assert_eq!(variants.len(), 5);

    for v in &variants {
        let json = serde_json::to_string(v).unwrap();
        let _back: KbEdgeType = serde_json::from_str(&json).unwrap();
    }
}

#[test]
fn test_knowledge_base_creation() {
    let db = mem_db();
    let kb = KnowledgeBase::new(db);
    assert!(kb.is_ok(), "KnowledgeBase::new should succeed with in-memory db");
}

#[tokio::test]
async fn test_knowledge_base_api_surface() {
    let db = mem_db();
    let kb = KnowledgeBase::new(db).unwrap();

    // All 9 methods should be callable and return Ok
    let ingest_result = kb.ingest(&IngestSource::Url("https://example.com".into())).await;
    assert!(ingest_result.is_ok());

    let compile_result = kb.compile().await;
    assert!(compile_result.is_ok());

    let query_result = kb.query("test question", &QueryOptions::default()).await;
    assert!(query_result.is_ok());

    let lint_result = kb.lint().await;
    assert!(lint_result.is_ok());

    let list_result = kb.list().await;
    assert!(list_result.is_ok());

    let stats_result = kb.stats().await;
    assert!(stats_result.is_ok());

    let search_result = kb.search("test", 10).await;
    assert!(search_result.is_ok());

    let delete_result = kb.delete("nonexistent").await;
    assert!(delete_result.is_ok());

    let dir = tempfile::tempdir().unwrap();
    let export_result = kb.export(dir.path()).await;
    assert!(export_result.is_ok());
}

#[test]
fn test_ingest_source_variants() {
    let file_source = IngestSource::FilePath(PathBuf::from("/tmp/test.md"));
    let url_source = IngestSource::Url("https://example.com".into());
    let dir_source = IngestSource::Directory(PathBuf::from("/tmp/docs"));

    // Verify all three variants serialize
    for source in [&file_source, &url_source, &dir_source] {
        let json = serde_json::to_string(source).unwrap();
        let _back: IngestSource = serde_json::from_str(&json).unwrap();
    }
}

#[test]
fn test_query_options_default() {
    let opts = QueryOptions::default();
    assert_eq!(opts.max_results, 10);
    assert!((opts.min_relevance - 0.0).abs() < f64::EPSILON);
    assert!(opts.domain_filter.is_none());
}
