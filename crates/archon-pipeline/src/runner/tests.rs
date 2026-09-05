use super::*;
use std::path::PathBuf;

// -- format_leann_results ------------------------------------------------

#[test]
fn format_leann_results_empty_input_returns_empty_string() {
    let results: Vec<archon_leann::SearchResult> = vec![];
    assert_eq!(format_leann_results(&results), "");
}

#[test]
fn format_leann_results_single_result_has_correct_markdown() {
    let results = vec![archon_leann::SearchResult {
        file_path: PathBuf::from("src/main.rs"),
        content: "fn main() {}".to_string(),
        language: "rust".to_string(),
        line_start: 1,
        line_end: 3,
        relevance_score: 0.95,
    }];

    let output = format_leann_results(&results);

    assert!(
        output.starts_with("## Code Context\n"),
        "should start with header"
    );
    assert!(output.contains("`src/main.rs`"), "should contain file path");
    assert!(output.contains("lines 1-3"), "should contain line range");
    assert!(output.contains("```rust"), "should contain language fence");
    assert!(
        output.contains("fn main() {}"),
        "should contain code content"
    );
}

#[test]
fn format_leann_results_multiple_results() {
    let results = vec![
        archon_leann::SearchResult {
            file_path: PathBuf::from("a.rs"),
            content: "fn a() {}".to_string(),
            language: "rust".to_string(),
            line_start: 1,
            line_end: 1,
            relevance_score: 0.9,
        },
        archon_leann::SearchResult {
            file_path: PathBuf::from("b.py"),
            content: "def b(): pass".to_string(),
            language: "python".to_string(),
            line_start: 10,
            line_end: 12,
            relevance_score: 0.7,
        },
    ];

    let output = format_leann_results(&results);

    // Both files should appear
    assert!(output.contains("`a.rs`"));
    assert!(output.contains("`b.py`"));
    assert!(output.contains("```rust"));
    assert!(output.contains("```python"));
}

// -- extract_modified_files ----------------------------------------------

#[test]
fn critical_agents_do_not_pass_on_final_low_quality_attempt() {
    assert!(!attempt_accepted(false, true, PIPELINE_MAX_ATTEMPTS));
}

#[test]
fn noncritical_agents_can_continue_after_final_low_quality_attempt() {
    assert!(attempt_accepted(false, false, PIPELINE_MAX_ATTEMPTS));
}

#[test]
fn threshold_pass_accepts_any_agent() {
    assert!(attempt_accepted(true, true, 1));
    assert!(attempt_accepted(true, false, 1));
}

#[test]
fn extract_modified_files_empty_log_returns_empty() {
    let log: Vec<ToolUseEntry> = vec![];
    assert!(extract_modified_files(&log).is_empty());
}

#[test]
fn extract_modified_files_extracts_write_and_edit() {
    let log = vec![
        ToolUseEntry {
            tool_name: "Write".to_string(),
            input: serde_json::json!({ "file_path": "/src/a.rs", "content": "..." }),
            output: serde_json::json!({}),
        },
        ToolUseEntry {
            tool_name: "Edit".to_string(),
            input: serde_json::json!({ "file_path": "/src/b.rs", "old_string": "x", "new_string": "y" }),
            output: serde_json::json!({}),
        },
    ];

    let paths = extract_modified_files(&log);
    assert_eq!(paths.len(), 2);
    assert_eq!(paths[0], PathBuf::from("/src/a.rs"));
    assert_eq!(paths[1], PathBuf::from("/src/b.rs"));
}

#[test]
fn extract_modified_files_deduplicates() {
    let log = vec![
        ToolUseEntry {
            tool_name: "Write".to_string(),
            input: serde_json::json!({ "file_path": "/src/a.rs", "content": "v1" }),
            output: serde_json::json!({}),
        },
        ToolUseEntry {
            tool_name: "Edit".to_string(),
            input: serde_json::json!({ "file_path": "/src/a.rs", "old_string": "x", "new_string": "y" }),
            output: serde_json::json!({}),
        },
    ];

    let paths = extract_modified_files(&log);
    assert_eq!(paths.len(), 1, "duplicate paths should be deduplicated");
    assert_eq!(paths[0], PathBuf::from("/src/a.rs"));
}

#[test]
fn extract_modified_files_ignores_other_tools() {
    let log = vec![
        ToolUseEntry {
            tool_name: "Read".to_string(),
            input: serde_json::json!({ "file_path": "/src/a.rs" }),
            output: serde_json::json!({}),
        },
        ToolUseEntry {
            tool_name: "Bash".to_string(),
            input: serde_json::json!({ "command": "ls" }),
            output: serde_json::json!({}),
        },
    ];

    assert!(extract_modified_files(&log).is_empty());
}

#[test]
fn extract_modified_files_skips_missing_file_path() {
    let log = vec![ToolUseEntry {
        tool_name: "Write".to_string(),
        input: serde_json::json!({ "content": "orphan content, no file_path" }),
        output: serde_json::json!({}),
    }];

    assert!(extract_modified_files(&log).is_empty());
}

// -- LeannIntegration (unit-level, no DB) --------------------------------

// NOTE: Full integration tests for LeannIntegration require a CozoDB
// instance and are covered in integration test files. Here we verify
// the helper functions that do not need a live DB.

#[test]
fn leann_integration_search_context_formats_correctly() {
    // This test exercises format_leann_results indirectly through the
    // struct method — we cannot construct a LeannIntegration without a
    // CodeIndex, but we can verify the formatting path separately.
    let results = vec![archon_leann::SearchResult {
        file_path: PathBuf::from("lib.rs"),
        content: "pub fn hello() {}".to_string(),
        language: "rust".to_string(),
        line_start: 5,
        line_end: 7,
        relevance_score: 0.85,
    }];

    let formatted = format_leann_results(&results);
    assert!(formatted.contains("## Code Context"));
    assert!(formatted.contains("lib.rs"));
    assert!(formatted.contains("lines 5-7"));
}
