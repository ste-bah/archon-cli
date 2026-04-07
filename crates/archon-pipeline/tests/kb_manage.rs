//! Tests for KB Management Commands (TASK-PIPE-D11).
//!
//! Validates: list, stats, search, delete (with cascade), export.

use std::collections::BTreeMap;
use std::path::Path;

use cozo::{DataValue, DbInstance, ScriptMutability};

use archon_pipeline::kb::schema::ensure_kb_schema;
use archon_pipeline::kb::KnowledgeBase;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn mem_db() -> DbInstance {
    DbInstance::new("mem", "", Default::default()).expect("in-memory CozoDB")
}

/// Insert a test node directly into CozoDB.
fn insert_node(db: &DbInstance, id: &str, node_type: &str, source: &str, title: &str, content: &str) {
    let hash = format!("hash_{}", id);
    let mut params = BTreeMap::new();
    params.insert("nid".to_string(), DataValue::from(id));
    params.insert("ntype".to_string(), DataValue::from(node_type));
    params.insert("src".to_string(), DataValue::from(source));
    params.insert("dtag".to_string(), DataValue::from("test"));
    params.insert("title".to_string(), DataValue::from(title));
    params.insert("content".to_string(), DataValue::from(content));
    params.insert("chash".to_string(), DataValue::from(hash.as_str()));
    params.insert("cidx".to_string(), DataValue::from(0i64));
    params.insert("cat".to_string(), DataValue::from(1000.0f64));
    params.insert("uat".to_string(), DataValue::from(1000.0f64));
    db.run_script(
        "?[node_id, node_type, source, domain_tag, title, content, content_hash, chunk_index, created_at, updated_at] \
         <- [[$nid, $ntype, $src, $dtag, $title, $content, $chash, $cidx, $cat, $uat]]
         :put kb_nodes { node_id => node_type, source, domain_tag, title, content, content_hash, chunk_index, created_at, updated_at }",
        params,
        ScriptMutability::Mutable,
    ).expect("insert test node");
}

/// Insert a test edge.
fn insert_edge(db: &DbInstance, edge_id: &str, src_id: &str, tgt_id: &str, edge_type: &str) {
    let mut params = BTreeMap::new();
    params.insert("eid".to_string(), DataValue::from(edge_id));
    params.insert("src".to_string(), DataValue::from(src_id));
    params.insert("tgt".to_string(), DataValue::from(tgt_id));
    params.insert("etype".to_string(), DataValue::from(edge_type));
    params.insert("cat".to_string(), DataValue::from(1000.0f64));
    db.run_script(
        "?[edge_id, source_node_id, target_node_id, edge_type, created_at] \
         <- [[$eid, $src, $tgt, $etype, $cat]]
         :put kb_edges { edge_id => source_node_id, target_node_id, edge_type, created_at }",
        params,
        ScriptMutability::Mutable,
    ).expect("insert test edge");
}

fn count_nodes(db: &DbInstance) -> usize {
    let result = db.run_script(
        "?[count(node_id)] := *kb_nodes{node_id}",
        Default::default(),
        ScriptMutability::Immutable,
    ).unwrap();
    result.rows[0][0].get_int().unwrap_or(0) as usize
}

fn count_edges(db: &DbInstance) -> usize {
    let result = db.run_script(
        "?[count(edge_id)] := *kb_edges{edge_id}",
        Default::default(),
        ScriptMutability::Immutable,
    ).unwrap();
    result.rows[0][0].get_int().unwrap_or(0) as usize
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

mod manage_tests {
    use super::*;

    fn test_kb(db: DbInstance) -> KnowledgeBase {
        KnowledgeBase::new(db).expect("kb creation")
    }

    #[tokio::test]
    async fn list_returns_all_nodes() {
        let db = mem_db();
        let kb = test_kb(db.clone());

        insert_node(&db, "n1", "raw", "/doc1.md", "Doc 1", "Content 1");
        insert_node(&db, "n2", "compiled", "/doc2.md", "Doc 2", "Content 2");
        insert_node(&db, "n3", "raw", "/doc3.md", "Doc 3", "Content 3");

        let nodes = kb.list().await.expect("list");
        assert_eq!(nodes.len(), 3, "should list all 3 nodes");
    }

    #[tokio::test]
    async fn list_empty_kb() {
        let db = mem_db();
        let kb = test_kb(db);

        let nodes = kb.list().await.expect("list");
        assert!(nodes.is_empty(), "empty KB should return no nodes");
    }

    #[tokio::test]
    async fn stats_counts_by_type() {
        let db = mem_db();
        let kb = test_kb(db.clone());

        insert_node(&db, "n1", "raw", "/a.md", "A", "a");
        insert_node(&db, "n2", "raw", "/b.md", "B", "b");
        insert_node(&db, "n3", "compiled", "/c.md", "C", "c");
        insert_edge(&db, "e1", "n1", "n3", "DerivedFrom");

        let stats = kb.stats().await.expect("stats");
        assert_eq!(stats.total_nodes, 3);
        assert_eq!(stats.total_edges, 1);
        assert_eq!(*stats.nodes_by_type.get("raw").unwrap_or(&0), 2);
        assert_eq!(*stats.nodes_by_type.get("compiled").unwrap_or(&0), 1);
    }

    #[tokio::test]
    async fn delete_removes_node() {
        let db = mem_db();
        let kb = test_kb(db.clone());

        insert_node(&db, "del1", "raw", "/d.md", "D", "d");
        assert_eq!(count_nodes(&db), 1);

        kb.delete("del1").await.expect("delete");
        assert_eq!(count_nodes(&db), 0, "node should be deleted");
    }

    #[tokio::test]
    async fn delete_cascades_edges() {
        let db = mem_db();
        let kb = test_kb(db.clone());

        insert_node(&db, "parent", "raw", "/p.md", "P", "p");
        insert_node(&db, "child", "compiled", "/c.md", "C", "c");
        insert_edge(&db, "e1", "child", "parent", "Provenance");
        insert_edge(&db, "e2", "parent", "child", "Provenance");

        assert_eq!(count_edges(&db), 2);

        kb.delete("parent").await.expect("delete parent");
        assert_eq!(count_nodes(&db), 1, "only child should remain");
        assert_eq!(count_edges(&db), 0, "all edges involving parent should be gone");
    }

    #[tokio::test]
    async fn delete_cascades_derived_nodes() {
        let db = mem_db();
        let kb = test_kb(db.clone());

        // Raw node -> compiled node derived from it
        insert_node(&db, "raw1", "raw", "/r.md", "Raw", "raw content");
        insert_node(&db, "comp1", "compiled", "/comp.md", "Compiled", "compiled content");
        insert_edge(&db, "e1", "comp1", "raw1", "DerivedFrom");

        assert_eq!(count_nodes(&db), 2);

        // Delete raw1 — should cascade to comp1 via DerivedFrom
        kb.delete("raw1").await.expect("delete raw1");
        assert_eq!(count_nodes(&db), 0, "derived node should also be deleted");
    }

    #[tokio::test]
    async fn export_creates_directory_structure() {
        let db = mem_db();
        let kb = test_kb(db.clone());

        insert_node(&db, "ex1", "raw", "/e.md", "Export Test", "Export content");

        let tmp = tempfile::tempdir().unwrap();
        let export_dir = tmp.path().join("export");

        kb.export(&export_dir).await.expect("export");

        // Should create the export directory
        assert!(export_dir.exists(), "export dir should exist");

        // Should have a raw/ subdirectory
        let raw_dir = export_dir.join("raw");
        assert!(raw_dir.exists(), "raw/ subdir should exist");

        // Should have at least one file
        let files: Vec<_> = std::fs::read_dir(&raw_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert!(!files.is_empty(), "raw/ should contain at least one exported file");

        // File should contain the content
        let file_content = std::fs::read_to_string(files[0].path()).unwrap();
        assert!(file_content.contains("Export content"), "exported file should contain node content");
        assert!(file_content.contains("node_id:"), "exported file should have frontmatter");
    }

    #[tokio::test]
    async fn delete_nonexistent_is_noop() {
        let db = mem_db();
        let kb = test_kb(db);

        // Should not error
        let result = kb.delete("nonexistent").await;
        assert!(result.is_ok(), "deleting nonexistent node should not error");
    }

    #[tokio::test]
    async fn search_finds_matching_nodes() {
        let db = mem_db();
        let kb = test_kb(db.clone());

        insert_node(&db, "s1", "raw", "/a.md", "Rust Guide", "Learn Rust programming");
        insert_node(&db, "s2", "raw", "/b.md", "Python Guide", "Learn Python scripting");
        insert_node(&db, "s3", "compiled", "/c.md", "Rust Advanced", "Advanced Rust topics");

        let results = kb.search("Rust", 10).await.expect("search");
        assert_eq!(results.len(), 2, "should find 2 nodes matching 'Rust'");

        let results = kb.search("Python", 10).await.expect("search");
        assert_eq!(results.len(), 1, "should find 1 node matching 'Python'");

        let results = kb.search("nonexistent_term", 10).await.expect("search");
        assert!(results.is_empty(), "should find no nodes for non-matching query");
    }
}
