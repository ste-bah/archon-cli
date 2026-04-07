//! Tests for the repository indexer (TASK-PIPE-D03).
//!
//! Validates: directory walking, language detection, chunking, embedding storage,
//! file hash change detection, schema idempotency, exclude patterns, and remove_file.

use std::collections::BTreeMap;
use std::path::Path;

use cozo::{DataValue, DbInstance, ScriptMutability};

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

/// Create an in-memory CozoDB instance for testing.
fn test_db() -> DbInstance {
    DbInstance::new("mem", "", Default::default()).expect("in-memory CozoDB")
}

/// Create a temp directory with a handful of Rust and Python source files.
fn create_test_repo(dir: &Path) {
    let src = dir.join("src");
    std::fs::create_dir_all(&src).unwrap();

    std::fs::write(
        src.join("main.rs"),
        "fn main() {\n    println!(\"hello\");\n}\n\nfn helper() -> i32 {\n    42\n}\n",
    )
    .unwrap();

    std::fs::write(
        src.join("lib.rs"),
        "pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n",
    )
    .unwrap();

    std::fs::write(
        src.join("utils.py"),
        "def greet(name):\n    return f\"Hello {name}\"\n\ndef farewell():\n    return \"Bye\"\n",
    )
    .unwrap();

    // A file that should be ignored
    let nm = dir.join("node_modules");
    std::fs::create_dir_all(&nm).unwrap();
    std::fs::write(nm.join("pkg.js"), "module.exports = {};").unwrap();

    // A non-code file
    std::fs::write(dir.join("README.md"), "# My Project\n").unwrap();
}

/// Count rows in the `code_chunks` relation.
fn count_chunks(db: &DbInstance) -> usize {
    let result = db
        .run_script(
            "?[count(chunk_id)] := *code_chunks{chunk_id}",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .expect("count query");
    result.rows[0][0].get_int().unwrap_or(0) as usize
}

/// Query chunks for a specific file path.
fn chunks_for_file(db: &DbInstance, file_path: &str) -> Vec<Vec<DataValue>> {
    let mut params = BTreeMap::new();
    params.insert("fp".to_string(), DataValue::from(file_path));
    let result = db
        .run_script(
            "?[chunk_id, language, line_start, line_end, file_hash] := \
             *code_chunks{chunk_id, file_path, language, line_start, line_end, file_hash}, \
             file_path = $fp",
            params,
            ScriptMutability::Immutable,
        )
        .expect("file query");
    result.rows
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

mod indexer_tests {
    use super::*;
    use archon_leann::indexer::{EmbeddingConfig, EmbeddingProviderKind, Indexer};
    use archon_leann::metadata::IndexConfig;

    #[test]
    fn ensure_schema_is_idempotent() {
        let db = test_db();
        let config = EmbeddingConfig {
            provider: EmbeddingProviderKind::Mock,
            dimension: 8,
        };
        let indexer = Indexer::new(db.clone(), config, None).expect("indexer creation");
        indexer.ensure_schema().expect("first ensure_schema");
        indexer.ensure_schema().expect("second ensure_schema should not error");
    }

    #[test]
    fn ensure_schema_creates_code_chunks_relation() {
        let db = test_db();
        let config = EmbeddingConfig {
            provider: EmbeddingProviderKind::Mock,
            dimension: 8,
        };
        let indexer = Indexer::new(db.clone(), config, None).expect("indexer creation");
        indexer.ensure_schema().unwrap();

        // Verify relation exists by querying it (empty result is fine)
        let result = db
            .run_script(
                "?[chunk_id] := *code_chunks{chunk_id}",
                Default::default(),
                ScriptMutability::Immutable,
            )
            .expect("code_chunks relation should exist");
        assert!(result.rows.is_empty());
    }

    #[tokio::test]
    async fn index_file_creates_chunks() {
        let db = test_db();
        let config = EmbeddingConfig {
            provider: EmbeddingProviderKind::Mock,
            dimension: 8,
        };
        let indexer = Indexer::new(db.clone(), config, None).expect("indexer creation");
        indexer.ensure_schema().unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let rs_file = tmp.path().join("example.rs");
        std::fs::write(
            &rs_file,
            "fn foo() -> i32 {\n    1\n}\n\nfn bar() -> i32 {\n    2\n}\n",
        )
        .unwrap();

        indexer.index_file(&rs_file).await.expect("index_file");

        let file_str = rs_file.to_string_lossy().to_string();
        let rows = chunks_for_file(&db, &file_str);
        assert!(
            rows.len() >= 2,
            "expected at least 2 chunks for 2 functions, got {}",
            rows.len()
        );
    }

    #[tokio::test]
    async fn index_file_unchanged_is_noop() {
        let db = test_db();
        let config = EmbeddingConfig {
            provider: EmbeddingProviderKind::Mock,
            dimension: 8,
        };
        let indexer = Indexer::new(db.clone(), config, None).expect("indexer creation");
        indexer.ensure_schema().unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let rs_file = tmp.path().join("noop.rs");
        std::fs::write(&rs_file, "fn only() -> bool {\n    true\n}\n").unwrap();

        indexer.index_file(&rs_file).await.unwrap();
        let count1 = count_chunks(&db);

        // Index again — same content, should be a no-op
        indexer.index_file(&rs_file).await.unwrap();
        let count2 = count_chunks(&db);
        assert_eq!(count1, count2, "unchanged file should not produce new chunks");
    }

    #[tokio::test]
    async fn index_file_changed_replaces_chunks() {
        let db = test_db();
        let config = EmbeddingConfig {
            provider: EmbeddingProviderKind::Mock,
            dimension: 8,
        };
        let indexer = Indexer::new(db.clone(), config, None).expect("indexer creation");
        indexer.ensure_schema().unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let rs_file = tmp.path().join("change.rs");
        std::fs::write(&rs_file, "fn one() -> i32 {\n    1\n}\n").unwrap();

        indexer.index_file(&rs_file).await.unwrap();
        let file_str = rs_file.to_string_lossy().to_string();
        let rows1 = chunks_for_file(&db, &file_str);
        assert_eq!(rows1.len(), 1);

        // Change file content — now two functions
        std::fs::write(
            &rs_file,
            "fn one() -> i32 {\n    1\n}\n\nfn two() -> i32 {\n    2\n}\n",
        )
        .unwrap();

        indexer.index_file(&rs_file).await.unwrap();
        let rows2 = chunks_for_file(&db, &file_str);
        assert_eq!(rows2.len(), 2, "changed file should replace old chunks");

        // Old chunk hash should be gone
        let old_hash = rows1[0][4].get_str().unwrap_or("").to_string();
        let new_hashes: Vec<String> = rows2
            .iter()
            .map(|r| r[4].get_str().unwrap_or("").to_string())
            .collect();
        assert!(
            !new_hashes.contains(&old_hash),
            "old file hash should be replaced"
        );
    }

    #[tokio::test]
    async fn remove_file_deletes_all_chunks() {
        let db = test_db();
        let config = EmbeddingConfig {
            provider: EmbeddingProviderKind::Mock,
            dimension: 8,
        };
        let indexer = Indexer::new(db.clone(), config, None).expect("indexer creation");
        indexer.ensure_schema().unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let rs_file = tmp.path().join("removeme.rs");
        std::fs::write(&rs_file, "fn gone() -> bool {\n    false\n}\n").unwrap();

        indexer.index_file(&rs_file).await.unwrap();
        assert!(count_chunks(&db) > 0);

        indexer.remove_file(&rs_file).await.unwrap();
        let file_str = rs_file.to_string_lossy().to_string();
        let rows = chunks_for_file(&db, &file_str);
        assert!(rows.is_empty(), "remove_file should delete all chunks");
    }

    #[tokio::test]
    async fn index_repository_walks_and_excludes() {
        let db = test_db();
        let config = EmbeddingConfig {
            provider: EmbeddingProviderKind::Mock,
            dimension: 8,
        };
        let indexer = Indexer::new(db.clone(), config, None).expect("indexer creation");
        indexer.ensure_schema().unwrap();

        let tmp = tempfile::tempdir().unwrap();
        create_test_repo(tmp.path());

        let index_config = IndexConfig {
            root_path: tmp.path().to_path_buf(),
            include_patterns: vec![],
            exclude_patterns: vec![
                "node_modules".to_string(),
                ".git".to_string(),
            ],
        };

        let stats = indexer
            .index_repository(tmp.path(), &index_config)
            .await
            .expect("index_repository");

        // Should have indexed main.rs, lib.rs, utils.py — NOT node_modules/pkg.js or README.md
        assert!(
            stats.total_files >= 3,
            "expected at least 3 files indexed, got {}",
            stats.total_files
        );
        assert!(
            stats.total_chunks >= 4,
            "expected at least 4 chunks (3 Rust fns + 2 Python fns), got {}",
            stats.total_chunks
        );

        // Verify languages are tracked
        assert!(
            stats.languages.contains_key("rust"),
            "stats should track rust files"
        );
        assert!(
            stats.languages.contains_key("python"),
            "stats should track python files"
        );

        // node_modules file should NOT be in the index
        let nm_file = tmp
            .path()
            .join("node_modules/pkg.js")
            .to_string_lossy()
            .to_string();
        let nm_rows = chunks_for_file(&db, &nm_file);
        assert!(nm_rows.is_empty(), "excluded files should not be indexed");
    }

    #[tokio::test]
    async fn index_repository_skips_non_code_files() {
        let db = test_db();
        let config = EmbeddingConfig {
            provider: EmbeddingProviderKind::Mock,
            dimension: 8,
        };
        let indexer = Indexer::new(db.clone(), config, None).expect("indexer creation");
        indexer.ensure_schema().unwrap();

        let tmp = tempfile::tempdir().unwrap();
        // Only non-code files
        std::fs::write(tmp.path().join("readme.txt"), "just text").unwrap();
        std::fs::write(tmp.path().join("data.csv"), "a,b,c").unwrap();
        std::fs::write(tmp.path().join("image.png"), &[0u8; 100]).unwrap();

        let index_config = IndexConfig {
            root_path: tmp.path().to_path_buf(),
            include_patterns: vec![],
            exclude_patterns: vec![],
        };

        let stats = indexer
            .index_repository(tmp.path(), &index_config)
            .await
            .unwrap();
        // txt files have no recognized language, should still be handled (or skipped gracefully)
        // but should not panic
        assert_eq!(stats.total_files, 0, "non-code files should not count as indexed");
    }

    #[test]
    fn hnsw_index_created_with_correct_dimension() {
        let db = test_db();

        // Test with dimension 8 (mock)
        let config = EmbeddingConfig {
            provider: EmbeddingProviderKind::Mock,
            dimension: 8,
        };
        let indexer = Indexer::new(db.clone(), config, None).expect("indexer creation");
        indexer.ensure_schema().unwrap();

        // Verify by inserting a vector of the right dimension
        let params = {
            let mut p = BTreeMap::new();
            p.insert("id".to_string(), DataValue::from("test-chunk"));
            p.insert("fp".to_string(), DataValue::from("/test.rs"));
            p.insert("lang".to_string(), DataValue::from("rust"));
            p.insert("ls".to_string(), DataValue::from(1i64));
            p.insert("le".to_string(), DataValue::from(5i64));
            p.insert("cc".to_string(), DataValue::from("fn test() {}"));
            p.insert("fh".to_string(), DataValue::from("abc123"));
            p.insert("ts".to_string(), DataValue::from(1234567890.0f64));
            let arr = ndarray::Array1::from_vec(vec![0.1f32; 8]);
            p.insert(
                "emb".to_string(),
                DataValue::Vec(cozo::Vector::F32(arr)),
            );
            p
        };

        let put = db.run_script(
            "?[chunk_id, file_path, language, line_start, line_end, chunk_content, file_hash, indexed_at, embedding] \
             <- [[$id, $fp, $lang, $ls, $le, $cc, $fh, $ts, $emb]]
             :put code_chunks { chunk_id => file_path, language, line_start, line_end, chunk_content, file_hash, indexed_at, embedding }",
            params,
            ScriptMutability::Mutable,
        );
        assert!(put.is_ok(), "should accept vectors of configured dimension: {:?}", put.err());
    }

    #[tokio::test]
    async fn index_repository_reports_language_counts() {
        let db = test_db();
        let config = EmbeddingConfig {
            provider: EmbeddingProviderKind::Mock,
            dimension: 8,
        };
        let indexer = Indexer::new(db.clone(), config, None).expect("indexer creation");
        indexer.ensure_schema().unwrap();

        let tmp = tempfile::tempdir().unwrap();
        create_test_repo(tmp.path());

        let index_config = IndexConfig {
            root_path: tmp.path().to_path_buf(),
            include_patterns: vec![],
            exclude_patterns: vec!["node_modules".to_string()],
        };

        let stats = indexer
            .index_repository(tmp.path(), &index_config)
            .await
            .unwrap();

        // Language breakdown: 2 rust files, 1 python file
        assert_eq!(
            stats.languages.get("rust").copied().unwrap_or(0),
            2,
            "should count 2 rust files"
        );
        assert_eq!(
            stats.languages.get("python").copied().unwrap_or(0),
            1,
            "should count 1 python file"
        );
    }
}
