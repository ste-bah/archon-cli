//! Tests for TASK-PIPE-D06: LEANN Pipeline Integration
//!
//! Verifies:
//! - format_leann_results produces correct markdown for empty + populated results
//! - extract_modified_files correctly parses Write/Edit tool entries
//! - extract_modified_files ignores non-modifying tools (Read, Bash, etc.)
//! - LeannIntegration.search_context returns empty string on no results

use std::path::PathBuf;

use archon_pipeline::runner::{
    extract_modified_files, format_leann_results, ToolUseEntry,
};

// ---------------------------------------------------------------------------
// format_leann_results tests
// ---------------------------------------------------------------------------

#[test]
fn test_format_leann_results_empty() {
    let results: Vec<archon_leann::SearchResult> = vec![];
    let formatted = format_leann_results(&results);
    assert!(
        formatted.is_empty(),
        "Empty results should produce an empty string, got: {:?}",
        formatted,
    );
}

#[test]
fn test_format_leann_results_formats_markdown() {
    let results = vec![
        archon_leann::SearchResult {
            file_path: PathBuf::from("src/foo.rs"),
            content: "fn example() { }".to_string(),
            language: "rust".to_string(),
            line_start: 10,
            line_end: 25,
            relevance_score: 0.92,
        },
        archon_leann::SearchResult {
            file_path: PathBuf::from("src/bar.py"),
            content: "def bar():\n    pass".to_string(),
            language: "python".to_string(),
            line_start: 5,
            line_end: 15,
            relevance_score: 0.78,
        },
    ];
    let formatted = format_leann_results(&results);

    // Should contain the header
    assert!(
        formatted.contains("## Code Context"),
        "Should contain '## Code Context' header",
    );

    // Should contain file paths with line ranges
    assert!(
        formatted.contains("`src/foo.rs` (lines 10-25)"),
        "Should contain foo.rs path with line range",
    );
    assert!(
        formatted.contains("`src/bar.py` (lines 5-15)"),
        "Should contain bar.py path with line range",
    );

    // Should contain the code content
    assert!(
        formatted.contains("fn example() { }"),
        "Should contain foo.rs content",
    );
    assert!(
        formatted.contains("def bar():"),
        "Should contain bar.py content",
    );

    // Should contain language-tagged code fences
    assert!(
        formatted.contains("```rust"),
        "Should contain rust code fence",
    );
    assert!(
        formatted.contains("```python"),
        "Should contain python code fence",
    );
}

// ---------------------------------------------------------------------------
// extract_modified_files tests
// ---------------------------------------------------------------------------

#[test]
fn test_extract_modified_files_from_write_tool() {
    let entries = vec![ToolUseEntry {
        tool_name: "Write".to_string(),
        input: serde_json::json!({
            "file_path": "/home/user/project/src/main.rs",
            "content": "fn main() {}"
        }),
        output: serde_json::json!({"status": "ok"}),
    }];
    let paths = extract_modified_files(&entries);
    assert_eq!(paths.len(), 1);
    assert_eq!(paths[0], PathBuf::from("/home/user/project/src/main.rs"));
}

#[test]
fn test_extract_modified_files_from_edit_tool() {
    let entries = vec![ToolUseEntry {
        tool_name: "Edit".to_string(),
        input: serde_json::json!({
            "file_path": "/home/user/project/src/lib.rs",
            "old_string": "old",
            "new_string": "new"
        }),
        output: serde_json::json!({"status": "ok"}),
    }];
    let paths = extract_modified_files(&entries);
    assert_eq!(paths.len(), 1);
    assert_eq!(paths[0], PathBuf::from("/home/user/project/src/lib.rs"));
}

#[test]
fn test_extract_modified_files_ignores_read_tool() {
    let entries = vec![
        ToolUseEntry {
            tool_name: "Read".to_string(),
            input: serde_json::json!({
                "file_path": "/home/user/project/src/read_only.rs"
            }),
            output: serde_json::json!({"content": "..."}),
        },
        ToolUseEntry {
            tool_name: "Bash".to_string(),
            input: serde_json::json!({
                "command": "ls -la"
            }),
            output: serde_json::json!({"output": "..."}),
        },
        ToolUseEntry {
            tool_name: "Grep".to_string(),
            input: serde_json::json!({
                "pattern": "foo",
                "path": "/tmp"
            }),
            output: serde_json::json!({"matches": []}),
        },
    ];
    let paths = extract_modified_files(&entries);
    assert!(
        paths.is_empty(),
        "Read/Bash/Grep tools should not produce modified file paths, got {:?}",
        paths,
    );
}

#[test]
fn test_extract_modified_files_deduplicates() {
    let entries = vec![
        ToolUseEntry {
            tool_name: "Edit".to_string(),
            input: serde_json::json!({ "file_path": "/src/main.rs", "old_string": "a", "new_string": "b" }),
            output: serde_json::json!({}),
        },
        ToolUseEntry {
            tool_name: "Edit".to_string(),
            input: serde_json::json!({ "file_path": "/src/main.rs", "old_string": "c", "new_string": "d" }),
            output: serde_json::json!({}),
        },
        ToolUseEntry {
            tool_name: "Write".to_string(),
            input: serde_json::json!({ "file_path": "/src/lib.rs", "content": "..." }),
            output: serde_json::json!({}),
        },
    ];
    let paths = extract_modified_files(&entries);
    assert_eq!(paths.len(), 2, "Duplicate paths should be deduplicated");
    assert!(paths.contains(&PathBuf::from("/src/main.rs")));
    assert!(paths.contains(&PathBuf::from("/src/lib.rs")));
}

#[test]
fn test_extract_modified_files_handles_missing_file_path() {
    let entries = vec![ToolUseEntry {
        tool_name: "Write".to_string(),
        input: serde_json::json!({ "content": "no file_path key" }),
        output: serde_json::json!({}),
    }];
    let paths = extract_modified_files(&entries);
    assert!(
        paths.is_empty(),
        "Missing file_path key should be skipped gracefully",
    );
}

#[test]
fn test_leann_context_empty_when_no_results() {
    // format_leann_results with empty vec should return ""
    let context = format_leann_results(&[]);
    assert_eq!(context, "", "No results should yield empty context string");
}
