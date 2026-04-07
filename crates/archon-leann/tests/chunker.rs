//! Integration tests for the symbol-aware code chunker (TASK-PIPE-D02).
//!
//! Tests cover: tree-sitter-based chunking for Rust, Python, TypeScript, Go;
//! line-based fallback for unknown grammars; grammar directory configuration;
//! content preservation; and the `available_languages()` API.

use archon_leann::chunker::{Chunker, Language};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Helper: build a default chunker with compiled-in grammars (no grammar_dir)
// ---------------------------------------------------------------------------
fn default_chunker() -> Chunker {
    Chunker::new(None).expect("Chunker::new(None) must succeed with built-in grammars")
}

// ===========================================================================
// 1. Rust: 5 functions => exactly 5 chunks, each with correct boundaries
// ===========================================================================

const RUST_FIVE_FUNCTIONS: &str = r#"fn alpha() -> i32 {
    1
}

fn beta(x: i32) -> i32 {
    x + 1
}

fn gamma() {
    println!("gamma");
}

fn delta(a: i32, b: i32) -> i32 {
    a * b
}

fn epsilon() -> String {
    String::from("hello")
}
"#;

#[test]
fn test_rust_five_functions_produces_five_chunks() {
    let chunker = default_chunker();
    let chunks = chunker.chunk_file(
        Path::new("example.rs"),
        RUST_FIVE_FUNCTIONS,
        Language::Rust,
    );
    assert_eq!(chunks.len(), 5, "Expected exactly 5 chunks for 5 Rust functions");
}

#[test]
fn test_rust_chunks_contain_complete_functions() {
    let chunker = default_chunker();
    let chunks = chunker.chunk_file(
        Path::new("example.rs"),
        RUST_FIVE_FUNCTIONS,
        Language::Rust,
    );
    let fn_names = ["alpha", "beta", "gamma", "delta", "epsilon"];
    for (i, name) in fn_names.iter().enumerate() {
        let content = &chunks[i].metadata.chunk_content;
        assert!(
            content.contains(&format!("fn {name}")),
            "Chunk {i} should contain function '{name}', got: {content}"
        );
    }
}

#[test]
fn test_rust_chunks_have_correct_line_ranges() {
    let chunker = default_chunker();
    let chunks = chunker.chunk_file(
        Path::new("example.rs"),
        RUST_FIVE_FUNCTIONS,
        Language::Rust,
    );
    for (i, chunk) in chunks.iter().enumerate() {
        assert!(
            chunk.metadata.line_start <= chunk.metadata.line_end,
            "Chunk {i}: line_start ({}) must be <= line_end ({})",
            chunk.metadata.line_start,
            chunk.metadata.line_end
        );
    }
    // Each function spans exactly 3 lines; ensure no overlap between consecutive chunks.
    for pair in chunks.windows(2) {
        assert!(
            pair[1].metadata.line_start > pair[0].metadata.line_end,
            "Chunks must not overlap: first ends at {}, second starts at {}",
            pair[0].metadata.line_end,
            pair[1].metadata.line_start
        );
    }
}

// ===========================================================================
// 2. Python: 1 class with 3 methods => at least 3 method-level chunks
// ===========================================================================

const PYTHON_CLASS_THREE_METHODS: &str = r#"class Calculator:
    def __init__(self, value):
        self.value = value

    def add(self, x):
        self.value += x
        return self

    def subtract(self, x):
        self.value -= x
        return self

    def result(self):
        return self.value
"#;

#[test]
fn test_python_class_produces_method_chunks() {
    let chunker = default_chunker();
    let chunks = chunker.chunk_file(
        Path::new("calculator.py"),
        PYTHON_CLASS_THREE_METHODS,
        Language::Python,
    );
    // Must produce at least 3 chunks so that each method is individually searchable.
    assert!(
        chunks.len() >= 3,
        "Expected at least 3 chunks for 3 Python methods, got {}",
        chunks.len()
    );
}

#[test]
fn test_python_methods_individually_searchable() {
    let chunker = default_chunker();
    let chunks = chunker.chunk_file(
        Path::new("calculator.py"),
        PYTHON_CLASS_THREE_METHODS,
        Language::Python,
    );
    let method_names = ["__init__", "add", "subtract", "result"];
    for name in method_names {
        let found = chunks.iter().any(|c| c.metadata.chunk_content.contains(&format!("def {name}")));
        assert!(found, "Method '{name}' must appear in at least one chunk");
    }
}

// ===========================================================================
// 3. TypeScript: mixed declarations => one chunk per declaration
// ===========================================================================

const TYPESCRIPT_MIXED: &str = r#"function greet(name: string): string {
    return `Hello, ${name}`;
}

class Logger {
    log(msg: string): void {
        console.log(msg);
    }
}

interface Config {
    host: string;
    port: number;
}
"#;

#[test]
fn test_typescript_mixed_declarations_chunk_count() {
    let chunker = default_chunker();
    let chunks = chunker.chunk_file(
        Path::new("app.ts"),
        TYPESCRIPT_MIXED,
        Language::TypeScript,
    );
    // Expect 3 chunks: function, class, interface
    assert_eq!(
        chunks.len(),
        3,
        "Expected 3 chunks for function + class + interface, got {}",
        chunks.len()
    );
}

#[test]
fn test_typescript_each_declaration_in_own_chunk() {
    let chunker = default_chunker();
    let chunks = chunker.chunk_file(
        Path::new("app.ts"),
        TYPESCRIPT_MIXED,
        Language::TypeScript,
    );
    let declarations = ["function greet", "class Logger", "interface Config"];
    for decl in declarations {
        let found = chunks.iter().any(|c| c.metadata.chunk_content.contains(decl));
        assert!(found, "Declaration '{decl}' must appear in a chunk");
    }
}

// ===========================================================================
// 4. Go: 2 functions + 1 method => 3 chunks
// ===========================================================================

const GO_TWO_FUNCS_ONE_METHOD: &str = r#"package main

import "fmt"

func Add(a, b int) int {
    return a + b
}

func Subtract(a, b int) int {
    return a - b
}

type Calculator struct {
    Value int
}

func (c *Calculator) Reset() {
    c.Value = 0
    fmt.Println("reset")
}
"#;

#[test]
fn test_go_two_functions_one_method_produces_three_chunks() {
    let chunker = default_chunker();
    let chunks = chunker.chunk_file(
        Path::new("main.go"),
        GO_TWO_FUNCS_ONE_METHOD,
        Language::Go,
    );
    assert_eq!(
        chunks.len(),
        3,
        "Expected 3 chunks for 2 Go functions + 1 method, got {}",
        chunks.len()
    );
}

#[test]
fn test_go_chunks_contain_expected_symbols() {
    let chunker = default_chunker();
    let chunks = chunker.chunk_file(
        Path::new("main.go"),
        GO_TWO_FUNCS_ONE_METHOD,
        Language::Go,
    );
    let symbols = ["func Add", "func Subtract", "func (c *Calculator) Reset"];
    for sym in symbols {
        let found = chunks.iter().any(|c| c.metadata.chunk_content.contains(sym));
        assert!(found, "Symbol '{sym}' must appear in a chunk");
    }
}

// ===========================================================================
// 5. Unknown language falls back to line-based chunking, max 100 lines
// ===========================================================================

#[test]
fn test_unknown_language_falls_back_to_line_based() {
    let chunker = default_chunker();
    // Generate a 250-line file in an unknown language.
    let content: String = (1..=250)
        .map(|i| format!("line {i}: data"))
        .collect::<Vec<_>>()
        .join("\n");
    let chunks = chunker.chunk_file(
        Path::new("data.xyz"),
        &content,
        Language::Unknown,
    );
    // Should produce at least 3 chunks (250 lines / 100 max).
    assert!(
        chunks.len() >= 3,
        "Expected at least 3 line-based chunks for 250 lines, got {}",
        chunks.len()
    );
}

#[test]
fn test_line_based_fallback_max_100_lines_per_chunk() {
    let chunker = default_chunker();
    let content: String = (1..=250)
        .map(|i| format!("line {i}: data"))
        .collect::<Vec<_>>()
        .join("\n");
    let chunks = chunker.chunk_file(
        Path::new("data.xyz"),
        &content,
        Language::Unknown,
    );
    for (i, chunk) in chunks.iter().enumerate() {
        let line_count = chunk.metadata.line_end - chunk.metadata.line_start + 1;
        assert!(
            line_count <= 100,
            "Chunk {i} has {line_count} lines, exceeding the 100-line max"
        );
    }
}

// ===========================================================================
// 6. Grammar directory configurable; missing directory does not crash
// ===========================================================================

#[test]
fn test_missing_grammar_directory_does_not_crash() {
    let result = Chunker::new(Some(PathBuf::from("/nonexistent/path")));
    assert!(
        result.is_ok(),
        "Chunker::new with a missing grammar directory must not crash: {:?}",
        result.err()
    );
}

#[test]
fn test_missing_grammar_dir_still_has_builtin_languages() {
    let chunker = Chunker::new(Some(PathBuf::from("/nonexistent/path")))
        .expect("must not crash");
    let langs = chunker.available_languages();
    // Built-in grammars should still be available.
    assert!(
        !langs.is_empty(),
        "Even with a bad grammar_dir, built-in languages must be available"
    );
}

// ===========================================================================
// 7. available_languages() returns only loaded languages
// ===========================================================================

#[test]
fn test_available_languages_with_builtins() {
    let chunker = default_chunker();
    let langs = chunker.available_languages();
    // At minimum, the four built-in grammars should be present.
    assert!(
        langs.len() >= 4,
        "Expected at least 4 built-in languages, got {}",
        langs.len()
    );
}

#[test]
fn test_available_languages_contains_rust() {
    let chunker = default_chunker();
    let langs = chunker.available_languages();
    assert!(
        langs.contains(&Language::Rust),
        "available_languages must include Rust"
    );
}

#[test]
fn test_available_languages_contains_python() {
    let chunker = default_chunker();
    let langs = chunker.available_languages();
    assert!(
        langs.contains(&Language::Python),
        "available_languages must include Python"
    );
}

#[test]
fn test_available_languages_contains_typescript() {
    let chunker = default_chunker();
    let langs = chunker.available_languages();
    assert!(
        langs.contains(&Language::TypeScript),
        "available_languages must include TypeScript"
    );
}

#[test]
fn test_available_languages_contains_go() {
    let chunker = default_chunker();
    let langs = chunker.available_languages();
    assert!(
        langs.contains(&Language::Go),
        "available_languages must include Go"
    );
}

// ===========================================================================
// 8. Chunks preserve exact source text (no truncation or modification)
// ===========================================================================

#[test]
fn test_chunks_preserve_exact_source_text() {
    let chunker = default_chunker();
    let chunks = chunker.chunk_file(
        Path::new("example.rs"),
        RUST_FIVE_FUNCTIONS,
        Language::Rust,
    );
    // Concatenating all chunk contents (with appropriate separators) should
    // account for every non-blank source line.
    let all_chunk_text: String = chunks
        .iter()
        .map(|c| c.metadata.chunk_content.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    // Every function body line must appear verbatim in the chunk output.
    for line in RUST_FIVE_FUNCTIONS.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        assert!(
            all_chunk_text.contains(trimmed),
            "Source line '{}' was not preserved in chunks",
            trimmed
        );
    }
}

#[test]
fn test_chunk_content_not_truncated() {
    let chunker = default_chunker();
    // Use a function with a longer body to ensure nothing is cut off.
    let rust_long_fn = r#"fn compute_stuff() -> Vec<i32> {
    let mut results = Vec::new();
    for i in 0..10 {
        results.push(i * i);
    }
    results.sort();
    results.dedup();
    results
}
"#;
    let chunks = chunker.chunk_file(Path::new("long.rs"), rust_long_fn, Language::Rust);
    assert_eq!(chunks.len(), 1);
    let content = &chunks[0].metadata.chunk_content;
    assert!(content.contains("results.dedup()"), "Body must not be truncated");
    assert!(content.contains("fn compute_stuff"), "Signature must be present");
}

// ===========================================================================
// 9. Line-based fallback splits at blank lines, max 100 lines
// ===========================================================================

#[test]
fn test_line_based_fallback_splits_at_blank_lines() {
    let chunker = default_chunker();
    // Two blocks of 5 lines separated by a blank line.
    let content = "aaa\nbbb\nccc\nddd\neee\n\nfff\nggg\nhhh\niii\njjj";
    let chunks = chunker.chunk_file(
        Path::new("notes.txt"),
        content,
        Language::Unknown,
    );
    // With blank-line splitting, we should get at least 2 chunks.
    assert!(
        chunks.len() >= 2,
        "Line-based fallback should split at blank lines; got {} chunk(s)",
        chunks.len()
    );
}

#[test]
fn test_line_based_fallback_respects_100_line_cap_across_blank_lines() {
    let chunker = default_chunker();
    // A single block with no blank lines that is 150 lines long.
    let content: String = (1..=150)
        .map(|i| format!("solid_line_{i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let chunks = chunker.chunk_file(
        Path::new("big.txt"),
        &content,
        Language::Unknown,
    );
    assert!(
        chunks.len() >= 2,
        "150-line block with no blank lines must still be split at 100-line cap, got {} chunk(s)",
        chunks.len()
    );
    for (i, chunk) in chunks.iter().enumerate() {
        let span = chunk.metadata.line_end - chunk.metadata.line_start + 1;
        assert!(
            span <= 100,
            "Chunk {i} spans {span} lines, exceeding the 100-line cap"
        );
    }
}

// ===========================================================================
// Additional edge-case tests
// ===========================================================================

#[test]
fn test_chunk_text_without_file_path() {
    let chunker = default_chunker();
    let code = "fn solo() { 42 }\n";
    let chunks = chunker.chunk_text(code, Language::Rust);
    assert!(
        !chunks.is_empty(),
        "chunk_text should produce at least one chunk"
    );
    assert!(
        chunks[0].metadata.chunk_content.contains("fn solo"),
        "chunk_text must preserve function text"
    );
}

#[test]
fn test_empty_file_produces_no_chunks() {
    let chunker = default_chunker();
    let chunks = chunker.chunk_file(Path::new("empty.rs"), "", Language::Rust);
    assert!(
        chunks.is_empty(),
        "An empty file should produce 0 chunks, got {}",
        chunks.len()
    );
}

#[test]
fn test_file_metadata_fields_populated() {
    let chunker = default_chunker();
    let chunks = chunker.chunk_file(
        Path::new("sample.rs"),
        "fn only() {}\n",
        Language::Rust,
    );
    assert_eq!(chunks.len(), 1);
    let meta = &chunks[0].metadata;
    assert_eq!(meta.file_path, PathBuf::from("sample.rs"));
    assert_eq!(meta.language, "rust");
    assert!(!meta.file_hash.is_empty(), "file_hash must be populated");
    assert!(meta.line_start >= 1, "line_start should be >= 1");
}

#[test]
fn test_embedding_vector_initialized() {
    let chunker = default_chunker();
    let chunks = chunker.chunk_file(
        Path::new("e.rs"),
        "fn e() {}\n",
        Language::Rust,
    );
    assert_eq!(chunks.len(), 1);
    // The chunker should initialize the embedding field (may be empty or zeroed
    // before the embedding pass, but it must exist).
    let _embedding = &chunks[0].embedding;
}
