//! Tests for KB Document Ingest (TASK-PIPE-D08).
//!
//! Validates: heading-aware markdown chunking, text paragraph splitting,
//! deduplication via content hash, batch embedding, directory ingest,
//! domain tags, and IngestResult reporting.

use std::collections::BTreeMap;

use cozo::{DataValue, DbInstance, ScriptMutability};

use archon_pipeline::kb::schema::ensure_kb_schema;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn mem_db() -> DbInstance {
    DbInstance::new("mem", "", Default::default()).expect("in-memory CozoDB")
}

/// Count rows in the `kb_nodes` relation.
fn count_nodes(db: &DbInstance) -> usize {
    let result = db
        .run_script(
            "?[count(node_id)] := *kb_nodes{node_id}",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .expect("count query");
    result.rows[0][0].get_int().unwrap_or(0) as usize
}

/// Query nodes by source path.
fn nodes_by_source(db: &DbInstance, source: &str) -> Vec<Vec<DataValue>> {
    let mut params = BTreeMap::new();
    params.insert("src".to_string(), DataValue::from(source));
    let result = db
        .run_script(
            "?[node_id, title, content_hash, chunk_index] := \
             *kb_nodes{node_id, source, title, content_hash, chunk_index}, \
             source = $src",
            params,
            ScriptMutability::Immutable,
        )
        .expect("source query");
    result.rows
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

mod ingest_tests {
    use super::*;
    use archon_pipeline::kb::ingest::Ingester;

    fn test_ingester(db: DbInstance) -> Ingester {
        ensure_kb_schema(&db).unwrap();
        Ingester::new(db).expect("ingester creation")
    }

    #[tokio::test]
    async fn markdown_heading_aware_chunking() {
        let db = mem_db();
        let ingester = test_ingester(db.clone());

        let tmp = tempfile::tempdir().unwrap();
        let md_file = tmp.path().join("doc.md");
        std::fs::write(
            &md_file,
            "# Introduction\n\nSome intro text.\n\n## Methods\n\nMethod details here.\n\n## Results\n\nResults text.\n\n## Conclusion\n\nFinal thoughts.\n",
        ).unwrap();

        let result = ingester
            .ingest_markdown(&md_file, "test")
            .await
            .expect("markdown ingest");

        // 4 headings = 4 chunks (Introduction, Methods, Results, Conclusion)
        assert!(
            result.chunks_processed >= 4,
            "expected >= 4 chunks for 4 headings, got {}",
            result.chunks_processed
        );
        assert!(result.errors.is_empty(), "no errors expected");
    }

    #[tokio::test]
    async fn text_paragraph_splitting() {
        let db = mem_db();
        let ingester = test_ingester(db.clone());

        let tmp = tempfile::tempdir().unwrap();
        let txt_file = tmp.path().join("doc.txt");
        // Each paragraph must be >= 200 chars to avoid merging. Distinct content to avoid dedup.
        let p1 = format!("First paragraph: {}", "alpha ".repeat(40));
        let p2 = format!("Second paragraph: {}", "beta ".repeat(40));
        let p3 = format!("Third paragraph: {}", "gamma ".repeat(40));
        let content = format!("{p1}\n\n{p2}\n\n{p3}\n");
        std::fs::write(&txt_file, &content).unwrap();

        let result = ingester
            .ingest_text(&txt_file, "test")
            .await
            .expect("text ingest");

        assert!(
            result.chunks_processed >= 3,
            "expected >= 3 chunks for 3 long paragraphs, got {}",
            result.chunks_processed
        );
    }

    #[tokio::test]
    async fn deduplication_skips_identical_content() {
        let db = mem_db();
        let ingester = test_ingester(db.clone());

        let tmp = tempfile::tempdir().unwrap();
        let md_file = tmp.path().join("dedup.md");
        std::fs::write(
            &md_file,
            "# Section One\n\nContent here.\n\n## Section Two\n\nMore content.\n",
        )
        .unwrap();

        let result1 = ingester.ingest_markdown(&md_file, "test").await.unwrap();
        assert!(result1.nodes_created > 0);

        // Ingest again — should detect duplicates
        let result2 = ingester.ingest_markdown(&md_file, "test").await.unwrap();
        assert_eq!(
            result2.nodes_created, 0,
            "duplicate ingest should create 0 new nodes, but created {}",
            result2.nodes_created
        );
    }

    #[tokio::test]
    async fn directory_ingest_recursive() {
        let db = mem_db();
        let ingester = test_ingester(db.clone());

        let tmp = tempfile::tempdir().unwrap();
        let sub = tmp.path().join("subdir");
        std::fs::create_dir_all(&sub).unwrap();

        std::fs::write(tmp.path().join("root.md"), "# Root\n\nRoot content.\n").unwrap();
        std::fs::write(sub.join("nested.md"), "# Nested\n\nNested content.\n").unwrap();
        // Non-text file should be skipped
        std::fs::write(tmp.path().join("binary.bin"), [0u8; 50]).unwrap();

        let result = ingester
            .ingest_directory(tmp.path(), "testdir", None)
            .await
            .expect("directory ingest");

        assert!(
            result.nodes_created >= 2,
            "expected >= 2 nodes from 2 markdown files, got {}",
            result.nodes_created
        );
    }

    #[tokio::test]
    async fn domain_tag_stored_correctly() {
        let db = mem_db();
        let ingester = test_ingester(db.clone());

        let tmp = tempfile::tempdir().unwrap();
        let md_file = tmp.path().join("tagged.md");
        std::fs::write(&md_file, "# Tagged\n\nTagged content.\n").unwrap();

        ingester
            .ingest_markdown(&md_file, "my-domain")
            .await
            .unwrap();

        // Query for domain_tag
        let result = db
            .run_script(
                "?[domain_tag] := *kb_nodes{domain_tag}, domain_tag = 'my-domain'",
                Default::default(),
                ScriptMutability::Immutable,
            )
            .expect("domain query");
        assert!(
            !result.rows.is_empty(),
            "should find nodes with domain_tag 'my-domain'"
        );
    }

    #[tokio::test]
    async fn ingest_result_reports_counts() {
        let db = mem_db();
        let ingester = test_ingester(db.clone());

        let tmp = tempfile::tempdir().unwrap();
        let md_file = tmp.path().join("counts.md");
        std::fs::write(
            &md_file,
            "# One\n\nFirst.\n\n## Two\n\nSecond.\n\n## Three\n\nThird.\n",
        )
        .unwrap();

        let result = ingester.ingest_markdown(&md_file, "test").await.unwrap();
        assert!(result.nodes_created > 0, "should report nodes created");
        assert!(
            result.chunks_processed > 0,
            "should report chunks processed"
        );
        assert!(result.errors.is_empty(), "should have no errors");
    }

    #[tokio::test]
    async fn content_hash_stored_per_node() {
        let db = mem_db();
        let ingester = test_ingester(db.clone());

        let tmp = tempfile::tempdir().unwrap();
        let md_file = tmp.path().join("hashed.md");
        std::fs::write(&md_file, "# Only\n\nSome content.\n").unwrap();

        ingester.ingest_markdown(&md_file, "test").await.unwrap();

        let result = db
            .run_script(
                "?[content_hash] := *kb_nodes{content_hash}",
                Default::default(),
                ScriptMutability::Immutable,
            )
            .unwrap();

        for row in &result.rows {
            let hash = row[0].get_str().unwrap_or("");
            assert!(!hash.is_empty(), "content_hash should not be empty");
            assert!(
                hash.len() == 64,
                "SHA-256 hex should be 64 chars, got {}",
                hash.len()
            );
        }
    }

    #[tokio::test]
    async fn chunk_index_preserves_order() {
        let db = mem_db();
        let ingester = test_ingester(db.clone());

        let tmp = tempfile::tempdir().unwrap();
        let md_file = tmp.path().join("ordered.md");
        std::fs::write(
            &md_file,
            "# First\n\nA.\n\n## Second\n\nB.\n\n## Third\n\nC.\n",
        )
        .unwrap();

        let src = md_file.to_string_lossy().to_string();
        ingester.ingest_markdown(&md_file, "test").await.unwrap();

        let rows = nodes_by_source(&db, &src);
        let mut indices: Vec<i64> = rows.iter().filter_map(|r| r[3].get_int()).collect();
        let sorted = {
            let mut s = indices.clone();
            s.sort();
            s
        };
        assert_eq!(indices.len(), sorted.len());
        // Indices should be sequential starting from 0
        indices.sort();
        for (i, idx) in indices.iter().enumerate() {
            assert_eq!(*idx, i as i64, "chunk_index should be sequential");
        }
    }

    #[tokio::test]
    async fn empty_file_produces_no_nodes() {
        let db = mem_db();
        let ingester = test_ingester(db.clone());

        let tmp = tempfile::tempdir().unwrap();
        let empty = tmp.path().join("empty.md");
        std::fs::write(&empty, "").unwrap();

        let result = ingester.ingest_markdown(&empty, "test").await.unwrap();
        assert_eq!(result.nodes_created, 0);
        assert_eq!(result.chunks_processed, 0);
    }
}
