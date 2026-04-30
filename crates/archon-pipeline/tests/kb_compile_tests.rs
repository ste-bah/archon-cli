//! Tests for KB LLM Compilation (TASK-PIPE-D09).
//!
//! Validates: no-op on empty KB, summary generation, concept extraction,
//! provenance edges, incremental compilation, LLM failure resilience,
//! index node creation, and CompileMetrics accuracy.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use cozo::{DataValue, DbInstance, ScriptMutability};

use archon_pipeline::kb::compile::{Compiler, KbLlmClient};
use archon_pipeline::kb::schema::ensure_kb_schema;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn mem_db() -> DbInstance {
    DbInstance::new("mem", "", Default::default()).expect("in-memory CozoDB")
}

fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

/// Insert a raw node directly into the database.
fn insert_raw_node(db: &DbInstance, node_id: &str, title: &str, content: &str) {
    let ts = now_secs();
    let mut params = BTreeMap::new();
    params.insert("nid".to_string(), DataValue::from(node_id));
    params.insert("title".to_string(), DataValue::from(title));
    params.insert("content".to_string(), DataValue::from(content));
    params.insert("ts".to_string(), DataValue::from(ts));

    db.run_script(
        "?[node_id, node_type, source, domain_tag, title, content, content_hash, chunk_index, created_at, updated_at] \
         <- [[$nid, 'raw', 'test', 'default', $title, $content, 'abc123', 0, $ts, $ts]] \
         :put kb_nodes { node_id => node_type, source, domain_tag, title, content, content_hash, chunk_index, created_at, updated_at }",
        params,
        ScriptMutability::Mutable,
    )
    .expect("insert raw node");
}

/// Count nodes of a given type.
fn count_nodes_of_type(db: &DbInstance, node_type: &str) -> usize {
    let mut params = BTreeMap::new();
    params.insert("nt".to_string(), DataValue::from(node_type));
    let result = db
        .run_script(
            "?[count(node_id)] := *kb_nodes{node_id, node_type}, node_type = $nt",
            params,
            ScriptMutability::Immutable,
        )
        .expect("count query");
    result.rows[0][0].get_int().unwrap_or(0) as usize
}

/// Count all edges of a given type.
fn count_edges_of_type(db: &DbInstance, edge_type: &str) -> usize {
    let mut params = BTreeMap::new();
    params.insert("et".to_string(), DataValue::from(edge_type));
    let result = db
        .run_script(
            "?[count(edge_id)] := *kb_edges{edge_id, edge_type}, edge_type = $et",
            params,
            ScriptMutability::Immutable,
        )
        .expect("count edges query");
    result.rows[0][0].get_int().unwrap_or(0) as usize
}

// ---------------------------------------------------------------------------
// Mock LLM clients
// ---------------------------------------------------------------------------

/// Mock that always returns a valid summary JSON response.
struct MockSummaryLlm;

#[async_trait::async_trait]
impl KbLlmClient for MockSummaryLlm {
    async fn complete(&self, prompt: &str) -> Result<String> {
        if prompt.contains("concepts")
            || prompt.contains("Concepts")
            || prompt.contains("Extract key concepts")
        {
            // Concept extraction response
            Ok(r#"[{"name": "TestConcept", "explanation": "A concept for testing.", "source_nodes": []}]"#.to_string())
        } else if prompt.contains("relationship") || prompt.contains("relationships") {
            // Cross-reference response
            Ok(r#"[]"#.to_string())
        } else {
            // Summary response
            Ok(r#"{"summary": "This is a test summary."}"#.to_string())
        }
    }
}

/// Mock that always fails (returns an error).
struct FailingLlm;

#[async_trait::async_trait]
impl KbLlmClient for FailingLlm {
    async fn complete(&self, _prompt: &str) -> Result<String> {
        Err(anyhow::anyhow!("LLM unavailable"))
    }
}

/// Mock that returns invalid (non-JSON) responses.
struct BadJsonLlm;

#[async_trait::async_trait]
impl KbLlmClient for BadJsonLlm {
    async fn complete(&self, _prompt: &str) -> Result<String> {
        Ok("not valid json at all".to_string())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

mod compile_tests {
    use super::*;

    fn setup_db() -> DbInstance {
        let db = mem_db();
        ensure_kb_schema(&db).unwrap();
        db
    }

    // Test 1: Compile with no raw nodes is a no-op
    #[tokio::test]
    async fn compile_empty_kb_returns_zeros() {
        let db = setup_db();
        let compiler = Compiler::new(db.clone(), Box::new(MockSummaryLlm)).unwrap();
        let metrics = compiler.compile().await.expect("compile");

        assert_eq!(metrics.summaries_generated, 0, "no summaries for empty KB");
        assert_eq!(metrics.concepts_extracted, 0, "no concepts for empty KB");
        assert_eq!(metrics.edges_created, 0, "no edges for empty KB");
        assert!(!metrics.index_updated, "index not updated for empty KB");
    }

    // Test 2: Compile produces summaries for raw nodes
    #[tokio::test]
    async fn compile_generates_summaries_for_raw_nodes() {
        let db = setup_db();
        insert_raw_node(
            &db,
            "node-001",
            "Test Document",
            "This is test content for the document.",
        );
        insert_raw_node(
            &db,
            "node-002",
            "Another Doc",
            "More content for another document.",
        );

        let compiler = Compiler::new(db.clone(), Box::new(MockSummaryLlm)).unwrap();
        let metrics = compiler.compile().await.expect("compile");

        assert_eq!(
            metrics.summaries_generated, 2,
            "should generate 2 summaries"
        );
        assert!(metrics.duration_secs >= 0.0);

        // Compiled nodes should exist
        let compiled_count = count_nodes_of_type(&db, "compiled");
        assert_eq!(compiled_count, 2, "should have 2 compiled nodes");
    }

    // Test 3: Compile extracts concepts
    #[tokio::test]
    async fn compile_extracts_concepts() {
        let db = setup_db();
        insert_raw_node(
            &db,
            "node-c01",
            "Concept Doc",
            "This document discusses important concepts.",
        );

        let compiler = Compiler::new(db.clone(), Box::new(MockSummaryLlm)).unwrap();
        let metrics = compiler.compile().await.expect("compile");

        assert!(
            metrics.concepts_extracted >= 1,
            "should extract at least 1 concept, got {}",
            metrics.concepts_extracted
        );

        let concept_count = count_nodes_of_type(&db, "concept");
        assert!(
            concept_count >= 1,
            "should have concept nodes in DB, got {}",
            concept_count
        );
    }

    // Test 4: Compile creates provenance edges from compiled nodes to source raw nodes
    #[tokio::test]
    async fn compile_creates_provenance_edges() {
        let db = setup_db();
        insert_raw_node(&db, "node-p01", "Provenance Doc", "Content to compile.");

        let compiler = Compiler::new(db.clone(), Box::new(MockSummaryLlm)).unwrap();
        compiler.compile().await.expect("compile");

        let provenance_count = count_edges_of_type(&db, "Provenance");
        assert!(
            provenance_count >= 1,
            "should have >= 1 provenance edge, got {}",
            provenance_count
        );
    }

    // Test 5: Incremental compilation — only processes nodes added after last compile
    #[tokio::test]
    async fn incremental_compile_only_processes_new_nodes() {
        let db = setup_db();
        insert_raw_node(&db, "node-i01", "First Doc", "Content of first document.");

        let compiler = Compiler::new(db.clone(), Box::new(MockSummaryLlm)).unwrap();

        // First compile
        let metrics1 = compiler.compile().await.expect("first compile");
        assert_eq!(metrics1.summaries_generated, 1, "first compile: 1 summary");

        // Second compile with no new nodes — should be no-op
        let metrics2 = compiler.compile().await.expect("second compile");
        assert_eq!(
            metrics2.summaries_generated, 0,
            "second compile: no new summaries, already compiled"
        );

        // Add a new node with a slightly later timestamp
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        insert_raw_node(&db, "node-i02", "Second Doc", "Content of second document.");

        // Third compile should only process the new node
        let metrics3 = compiler.compile().await.expect("third compile");
        assert_eq!(
            metrics3.summaries_generated, 1,
            "third compile: only 1 new summary"
        );
    }

    // Test 6: LLM parse failure is logged and skipped (not fatal)
    #[tokio::test]
    async fn llm_parse_failure_is_not_fatal() {
        let db = setup_db();
        insert_raw_node(&db, "node-b01", "Bad JSON Doc", "Some content.");

        let compiler = Compiler::new(db.clone(), Box::new(BadJsonLlm)).unwrap();
        // Should not panic or return an error — just log and continue
        let result = compiler.compile().await;
        assert!(
            result.is_ok(),
            "compile should succeed even when LLM returns bad JSON"
        );

        // The compiled node should still be created (using raw response as fallback)
        let compiled_count = count_nodes_of_type(&db, "compiled");
        assert!(
            compiled_count >= 1,
            "should still create compiled node with fallback content"
        );
    }

    // Test 7: Index node created/updated after compile
    #[tokio::test]
    async fn compile_creates_index_node() {
        let db = setup_db();
        insert_raw_node(&db, "node-ix01", "Index Doc", "Content for index.");

        let compiler = Compiler::new(db.clone(), Box::new(MockSummaryLlm)).unwrap();
        let metrics = compiler.compile().await.expect("compile");

        assert!(
            metrics.index_updated,
            "index should be updated after compile"
        );

        let index_count = count_nodes_of_type(&db, "index");
        assert_eq!(index_count, 1, "should have exactly 1 index node");
    }

    // Test 8: CompileResult has correct counts
    #[tokio::test]
    async fn compile_metrics_counts_are_accurate() {
        let db = setup_db();
        insert_raw_node(&db, "node-m01", "Metrics Doc 1", "Content one.");
        insert_raw_node(&db, "node-m02", "Metrics Doc 2", "Content two.");
        insert_raw_node(&db, "node-m03", "Metrics Doc 3", "Content three.");

        let compiler = Compiler::new(db.clone(), Box::new(MockSummaryLlm)).unwrap();
        let metrics = compiler.compile().await.expect("compile");

        assert_eq!(
            metrics.summaries_generated, 3,
            "should generate 3 summaries"
        );
        assert!(
            metrics.edges_created >= 3,
            "should have >= 3 provenance edges (one per compiled node)"
        );
        assert!(
            metrics.duration_secs >= 0.0,
            "duration should be non-negative"
        );
    }

    // Test 9: compile_document produces correct DocumentCompilation
    #[tokio::test]
    async fn compile_document_returns_correct_structure() {
        use archon_pipeline::kb::KbNodeType;
        use archon_pipeline::kb::schema::KbNode;

        let db = setup_db();
        let compiler = Compiler::new(db.clone(), Box::new(MockSummaryLlm)).unwrap();

        let node = KbNode {
            node_id: "node-d01".to_string(),
            node_type: KbNodeType::Raw,
            source: "test".to_string(),
            domain_tag: "default".to_string(),
            title: "Test Node".to_string(),
            content: "Test content here.".to_string(),
            content_hash: "abc".to_string(),
            chunk_index: 0,
            created_at: now_secs(),
            updated_at: now_secs(),
        };

        let result = compiler
            .compile_document(&node)
            .await
            .expect("compile_document");
        assert!(
            !result.node_id.is_empty(),
            "compiled node_id should not be empty"
        );
        assert!(!result.summary.is_empty(), "summary should not be empty");
    }
}
