//! Public API tests for archon-leann crate.

use archon_leann::{CodeIndex, CodeMetadata, IndexConfig, IndexStats, QueueResult, SearchResult};
use std::path::{Path, PathBuf};

#[tokio::test]
async fn test_code_index_creation() {
    let db = cozo::DbInstance::new("mem", "", Default::default()).unwrap();
    let config = archon_leann::indexer::EmbeddingConfig {
        provider: archon_leann::indexer::EmbeddingProviderKind::Mock,
        dimension: 8,
    };
    let idx = CodeIndex::from_db(db, config).unwrap();
    // from_db sets db_path to empty
    assert_eq!(idx.db_path(), Path::new(""));
}

#[tokio::test]
async fn test_search_code_returns_empty() {
    let db = cozo::DbInstance::new("mem", "", Default::default()).unwrap();
    let config = archon_leann::indexer::EmbeddingConfig {
        provider: archon_leann::indexer::EmbeddingProviderKind::Mock,
        dimension: 8,
    };
    let idx = CodeIndex::from_db(db, config).unwrap();
    let results = idx.search_code("fn main", 10).unwrap();
    assert!(results.is_empty());
}

#[tokio::test]
async fn test_index_repository_returns_default_stats() {
    let tmp = tempfile::tempdir().unwrap();
    let db = cozo::DbInstance::new("mem", "", Default::default()).unwrap();
    let config = archon_leann::indexer::EmbeddingConfig {
        provider: archon_leann::indexer::EmbeddingProviderKind::Mock,
        dimension: 8,
    };
    let idx = CodeIndex::from_db(db, config).unwrap();
    let config = IndexConfig {
        root_path: tmp.path().to_path_buf(),
        include_patterns: vec![],
        exclude_patterns: vec![],
    };
    let stats = idx.index_repository(tmp.path(), &config).await.unwrap();
    assert_eq!(stats.total_files, 0);
    assert_eq!(stats.total_chunks, 0);
    assert_eq!(stats.index_size_bytes, 0);
    assert!(stats.languages.is_empty());
}

#[tokio::test]
async fn test_index_file_succeeds() {
    let tmp = tempfile::tempdir().unwrap();
    let rs_file = tmp.path().join("foo.rs");
    std::fs::write(&rs_file, "fn main() {}\n").unwrap();

    let db = cozo::DbInstance::new("mem", "", Default::default()).unwrap();
    let config = archon_leann::indexer::EmbeddingConfig {
        provider: archon_leann::indexer::EmbeddingProviderKind::Mock,
        dimension: 8,
    };
    let idx = CodeIndex::from_db(db, config).unwrap();
    let result = idx.index_file(&rs_file).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_find_similar_code_returns_empty() {
    let db = cozo::DbInstance::new("mem", "", Default::default()).unwrap();
    let config = archon_leann::indexer::EmbeddingConfig {
        provider: archon_leann::indexer::EmbeddingProviderKind::Mock,
        dimension: 8,
    };
    let idx = CodeIndex::from_db(db, config).unwrap();
    let results = idx.find_similar_code("let x = 42;", 5).unwrap();
    assert!(results.is_empty());
}

#[tokio::test]
async fn test_process_queue_returns_default() {
    let db = cozo::DbInstance::new("mem", "", Default::default()).unwrap();
    let config = archon_leann::indexer::EmbeddingConfig {
        provider: archon_leann::indexer::EmbeddingProviderKind::Mock,
        dimension: 8,
    };
    let idx = CodeIndex::from_db(db, config).unwrap();
    let qr = idx.process_queue(Path::new("/tmp/queue")).await.unwrap();
    assert_eq!(qr.processed, 0);
    assert_eq!(qr.failed, 0);
    assert_eq!(qr.remaining, 0);
}

#[tokio::test]
async fn test_stats_returns_default() {
    let db = cozo::DbInstance::new("mem", "", Default::default()).unwrap();
    let config = archon_leann::indexer::EmbeddingConfig {
        provider: archon_leann::indexer::EmbeddingProviderKind::Mock,
        dimension: 8,
    };
    let idx = CodeIndex::from_db(db, config).unwrap();
    let stats = idx.stats().await.unwrap();
    assert_eq!(stats.total_files, 0);
    assert_eq!(stats.total_chunks, 0);
}

#[test]
fn test_detect_language() {
    use archon_leann::language::detect_language;

    assert_eq!(
        detect_language(Path::new("foo.rs")),
        Some("rust".to_string())
    );
    assert_eq!(
        detect_language(Path::new("bar.py")),
        Some("python".to_string())
    );
    assert_eq!(
        detect_language(Path::new("baz.ts")),
        Some("typescript".to_string())
    );
    assert_eq!(
        detect_language(Path::new("qux.tsx")),
        Some("typescriptreact".to_string())
    );
    assert_eq!(
        detect_language(Path::new("app.js")),
        Some("javascript".to_string())
    );
    assert_eq!(
        detect_language(Path::new("app.jsx")),
        Some("javascriptreact".to_string())
    );
    assert_eq!(
        detect_language(Path::new("main.go")),
        Some("go".to_string())
    );
    assert_eq!(
        detect_language(Path::new("Main.java")),
        Some("java".to_string())
    );
    assert_eq!(detect_language(Path::new("foo.c")), Some("c".to_string()));
    assert_eq!(
        detect_language(Path::new("foo.cpp")),
        Some("cpp".to_string())
    );
    assert_eq!(detect_language(Path::new("foo.h")), Some("c".to_string()));
    assert_eq!(
        detect_language(Path::new("foo.hpp")),
        Some("cpp".to_string())
    );
    assert_eq!(
        detect_language(Path::new("foo.rb")),
        Some("ruby".to_string())
    );
    assert_eq!(
        detect_language(Path::new("foo.php")),
        Some("php".to_string())
    );
    assert_eq!(
        detect_language(Path::new("foo.swift")),
        Some("swift".to_string())
    );
    assert_eq!(
        detect_language(Path::new("foo.kt")),
        Some("kotlin".to_string())
    );
    assert_eq!(
        detect_language(Path::new("foo.scala")),
        Some("scala".to_string())
    );
    assert_eq!(
        detect_language(Path::new("foo.cs")),
        Some("csharp".to_string())
    );
    assert_eq!(
        detect_language(Path::new("foo.lua")),
        Some("lua".to_string())
    );
    assert_eq!(
        detect_language(Path::new("foo.sh")),
        Some("shell".to_string())
    );
    assert_eq!(
        detect_language(Path::new("foo.sql")),
        Some("sql".to_string())
    );
    assert_eq!(
        detect_language(Path::new("foo.md")),
        Some("markdown".to_string())
    );
    assert_eq!(
        detect_language(Path::new("foo.json")),
        Some("json".to_string())
    );
    assert_eq!(
        detect_language(Path::new("foo.yaml")),
        Some("yaml".to_string())
    );
    assert_eq!(
        detect_language(Path::new("foo.yml")),
        Some("yaml".to_string())
    );
    assert_eq!(
        detect_language(Path::new("foo.toml")),
        Some("toml".to_string())
    );
    assert_eq!(detect_language(Path::new("foo.xyz_unknown")), None);
}

#[test]
fn test_is_excluded() {
    use archon_leann::language::{default_exclude_patterns, is_excluded};

    let patterns = default_exclude_patterns();
    assert!(is_excluded(
        Path::new("project/node_modules/foo.js"),
        &patterns
    ));
    assert!(is_excluded(
        Path::new("project/target/debug/main"),
        &patterns
    ));
    assert!(is_excluded(Path::new("project/.git/config"), &patterns));
    assert!(is_excluded(
        Path::new("project/__pycache__/mod.pyc"),
        &patterns
    ));
    assert!(!is_excluded(Path::new("project/src/main.rs"), &patterns));
    assert!(!is_excluded(Path::new("project/lib/util.py"), &patterns));
}

#[test]
fn test_default_exclude_patterns() {
    use archon_leann::language::default_exclude_patterns;

    let patterns = default_exclude_patterns();
    assert!(
        patterns.len() >= 7,
        "Expected at least 7 patterns, got {}",
        patterns.len()
    );

    let joined = patterns.join(" ");
    assert!(joined.contains("node_modules"), "missing node_modules");
    assert!(joined.contains("target"), "missing target");
    assert!(joined.contains(".git"), "missing .git");
    assert!(joined.contains("__pycache__"), "missing __pycache__");
}

#[test]
fn test_search_result_fields() {
    let sr = SearchResult {
        file_path: PathBuf::from("src/main.rs"),
        content: "fn main() {}".to_string(),
        language: "rust".to_string(),
        line_start: 1,
        line_end: 1,
        relevance_score: 0.95,
    };
    assert_eq!(sr.file_path, PathBuf::from("src/main.rs"));
    assert_eq!(sr.content, "fn main() {}");
    assert_eq!(sr.language, "rust");
    assert_eq!(sr.line_start, 1);
    assert_eq!(sr.line_end, 1);
    assert!((sr.relevance_score - 0.95).abs() < f64::EPSILON);
}

#[test]
fn test_index_stats_display() {
    let stats = IndexStats::default();
    let display = format!("{}", stats);
    assert!(!display.is_empty(), "Display output should not be empty");
}

#[test]
fn test_no_mcp_references() {
    // Scan source directory for "mcp" references
    let src_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut found_mcp = Vec::new();

    fn scan_dir(dir: &Path, found: &mut Vec<String>) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() && path.extension().is_some_and(|e| e == "rs") {
                    if let Ok(content) = std::fs::read_to_string(&path)
                        && content.to_lowercase().contains("mcp")
                    {
                        found.push(path.display().to_string());
                    }
                } else if path.is_dir() {
                    scan_dir(&path, found);
                }
            }
        }
    }

    scan_dir(&src_dir, &mut found_mcp);
    assert!(
        found_mcp.is_empty(),
        "Found MCP references in source files: {:?}",
        found_mcp
    );
}

#[test]
fn test_public_api_surface() {
    // Verify all public types are accessible from the root
    // CodeIndex::new now requires EmbeddingConfig + returns Result
    let _ = |db: cozo::DbInstance| {
        let config = archon_leann::indexer::EmbeddingConfig::default();
        CodeIndex::from_db(db, config)
    };
    let _sr = SearchResult {
        file_path: PathBuf::new(),
        content: String::new(),
        language: String::new(),
        line_start: 0,
        line_end: 0,
        relevance_score: 0.0,
    };
    let _ic = IndexConfig {
        root_path: PathBuf::new(),
        include_patterns: vec![],
        exclude_patterns: vec![],
    };
    let _is = IndexStats::default();
    let _qr = QueueResult::default();
    let _cm = CodeMetadata {
        file_path: PathBuf::new(),
        language: String::new(),
        line_start: 0,
        line_end: 0,
        chunk_content: String::new(),
        file_hash: String::new(),
    };
}
