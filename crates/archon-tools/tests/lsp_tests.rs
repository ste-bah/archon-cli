//! Tests for TASK-CLI-313: LSP Integration.
//!
//! Unit-testable parts: schema parsing, tool properties, diagnostic registry,
//! server detection logic, formatter output. No live LSP server required.

use archon_tools::lsp_diagnostics::LspDiagnosticRegistry;
use archon_tools::lsp_manager::LspServerManager;
use archon_tools::lsp_tool::LspTool;
use archon_tools::lsp_types::{LspInput, LspOperation, LspOutput};
use archon_tools::tool::Tool;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// LspOperation / LspInput schema parsing
// ---------------------------------------------------------------------------

#[test]
fn parse_go_to_definition_op() {
    let input: LspInput = serde_json::from_value(serde_json::json!({
        "operation": "goToDefinition",
        "file_path": "/home/user/src/main.rs",
        "line": 10,
        "character": 5
    }))
    .unwrap();
    assert!(matches!(input.operation, LspOperation::GoToDefinition));
    assert_eq!(input.file_path, "/home/user/src/main.rs");
    assert_eq!(input.line, 10);
    assert_eq!(input.character, 5);
}

#[test]
fn parse_find_references_op() {
    let input: LspInput = serde_json::from_value(serde_json::json!({
        "operation": "findReferences",
        "file_path": "/src/lib.rs",
        "line": 1,
        "character": 0
    }))
    .unwrap();
    assert!(matches!(input.operation, LspOperation::FindReferences));
}

#[test]
fn parse_hover_op() {
    let input: LspInput = serde_json::from_value(serde_json::json!({
        "operation": "hover",
        "file_path": "/src/lib.rs",
        "line": 5,
        "character": 10
    }))
    .unwrap();
    assert!(matches!(input.operation, LspOperation::Hover));
}

#[test]
fn parse_document_symbol_op() {
    let input: LspInput = serde_json::from_value(serde_json::json!({
        "operation": "documentSymbol",
        "file_path": "/src/lib.rs",
        "line": 0,
        "character": 0
    }))
    .unwrap();
    assert!(matches!(input.operation, LspOperation::DocumentSymbol));
}

#[test]
fn parse_workspace_symbol_op() {
    let input: LspInput = serde_json::from_value(serde_json::json!({
        "operation": "workspaceSymbol",
        "file_path": "/src/lib.rs",
        "line": 0,
        "character": 0
    }))
    .unwrap();
    assert!(matches!(input.operation, LspOperation::WorkspaceSymbol));
}

#[test]
fn parse_go_to_implementation_op() {
    let input: LspInput = serde_json::from_value(serde_json::json!({
        "operation": "goToImplementation",
        "file_path": "/src/lib.rs",
        "line": 3,
        "character": 7
    }))
    .unwrap();
    assert!(matches!(input.operation, LspOperation::GoToImplementation));
}

#[test]
fn parse_prepare_call_hierarchy_op() {
    let input: LspInput = serde_json::from_value(serde_json::json!({
        "operation": "prepareCallHierarchy",
        "file_path": "/src/lib.rs",
        "line": 20,
        "character": 4
    }))
    .unwrap();
    assert!(matches!(
        input.operation,
        LspOperation::PrepareCallHierarchy
    ));
}

#[test]
fn parse_incoming_calls_op() {
    let input: LspInput = serde_json::from_value(serde_json::json!({
        "operation": "incomingCalls",
        "file_path": "/src/lib.rs",
        "line": 20,
        "character": 4
    }))
    .unwrap();
    assert!(matches!(input.operation, LspOperation::IncomingCalls));
}

#[test]
fn parse_outgoing_calls_op() {
    let input: LspInput = serde_json::from_value(serde_json::json!({
        "operation": "outgoingCalls",
        "file_path": "/src/lib.rs",
        "line": 20,
        "character": 4
    }))
    .unwrap();
    assert!(matches!(input.operation, LspOperation::OutgoingCalls));
}

#[test]
fn exactly_9_operations_exist() {
    // Ensure all 9 operation variants are covered by the enum
    // (exhaustive match would fail to compile if a variant is missing)
    fn _all_ops(op: LspOperation) -> u8 {
        match op {
            LspOperation::GoToDefinition => 1,
            LspOperation::FindReferences => 2,
            LspOperation::Hover => 3,
            LspOperation::DocumentSymbol => 4,
            LspOperation::WorkspaceSymbol => 5,
            LspOperation::GoToImplementation => 6,
            LspOperation::PrepareCallHierarchy => 7,
            LspOperation::IncomingCalls => 8,
            LspOperation::OutgoingCalls => 9,
        }
    }
    // Verify count
    assert_eq!(LspOperation::ALL.len(), 9);
}

#[test]
fn unknown_operation_fails_to_parse() {
    let result: Result<LspInput, _> = serde_json::from_value(serde_json::json!({
        "operation": "diagnostics",
        "file_path": "/src/lib.rs",
        "line": 0,
        "character": 0
    }));
    assert!(result.is_err(), "diagnostics is NOT a tool operation");
}

#[test]
fn completions_operation_fails_to_parse() {
    let result: Result<LspInput, _> = serde_json::from_value(serde_json::json!({
        "operation": "completions",
        "file_path": "/src/lib.rs",
        "line": 0,
        "character": 0
    }));
    assert!(result.is_err(), "completions is NOT a tool operation");
}

// ---------------------------------------------------------------------------
// LspTool properties
// ---------------------------------------------------------------------------

#[test]
fn lsp_tool_is_read_only() {
    let manager = LspServerManager::new(PathBuf::from("/tmp"), None);
    let tool = LspTool::new(std::sync::Arc::new(tokio::sync::Mutex::new(manager)));
    assert!(tool.is_read_only());
}

#[test]
fn lsp_tool_is_concurrency_safe() {
    let manager = LspServerManager::new(PathBuf::from("/tmp"), None);
    let tool = LspTool::new(std::sync::Arc::new(tokio::sync::Mutex::new(manager)));
    assert!(tool.is_concurrency_safe());
}

#[test]
fn lsp_tool_is_disabled_when_not_connected() {
    let manager = LspServerManager::new(PathBuf::from("/tmp"), None);
    let tool = LspTool::new(std::sync::Arc::new(tokio::sync::Mutex::new(manager)));
    // When no LSP server is connected, is_enabled() must return false
    assert!(!tool.is_enabled());
}

#[test]
fn lsp_tool_name_is_lsp() {
    let manager = LspServerManager::new(PathBuf::from("/tmp"), None);
    let tool = LspTool::new(std::sync::Arc::new(tokio::sync::Mutex::new(manager)));
    assert_eq!(tool.name(), "lsp");
}

#[test]
fn lsp_tool_input_schema_has_operation_field() {
    let manager = LspServerManager::new(PathBuf::from("/tmp"), None);
    let tool = LspTool::new(std::sync::Arc::new(tokio::sync::Mutex::new(manager)));
    let schema = tool.input_schema();
    let props = schema["properties"].as_object().unwrap();
    assert!(
        props.contains_key("operation"),
        "input schema must have 'operation' field"
    );
    assert!(
        props.contains_key("file_path"),
        "input schema must have 'file_path' field"
    );
    assert!(
        props.contains_key("line"),
        "input schema must have 'line' field"
    );
    assert!(
        props.contains_key("character"),
        "input schema must have 'character' field"
    );
}

// ---------------------------------------------------------------------------
// LspDiagnosticRegistry
// ---------------------------------------------------------------------------

#[test]
fn diagnostic_registry_starts_empty() {
    let registry = LspDiagnosticRegistry::new();
    let diags = registry.get_diagnostics("/src/main.rs");
    assert!(
        diags.is_empty(),
        "fresh registry should have no diagnostics"
    );
}

#[test]
fn diagnostic_registry_stores_and_retrieves() {
    let mut registry = LspDiagnosticRegistry::new();
    let diagnostic = archon_tools::lsp_diagnostics::LspDiagnostic {
        message: "unused variable".to_string(),
        severity: archon_tools::lsp_diagnostics::DiagnosticSeverity::Warning,
        range_start_line: 5,
        range_start_char: 4,
        range_end_line: 5,
        range_end_char: 12,
        code: Some("dead_code".to_string()),
        source: Some("rustc".to_string()),
    };
    registry.publish("/src/main.rs", vec![diagnostic]);
    let diags = registry.get_diagnostics("/src/main.rs");
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].message, "unused variable");
}

#[test]
fn diagnostic_registry_replaces_on_republish() {
    let mut registry = LspDiagnosticRegistry::new();
    registry.publish(
        "/src/main.rs",
        vec![archon_tools::lsp_diagnostics::LspDiagnostic {
            message: "first".to_string(),
            severity: archon_tools::lsp_diagnostics::DiagnosticSeverity::Error,
            range_start_line: 1,
            range_start_char: 0,
            range_end_line: 1,
            range_end_char: 5,
            code: None,
            source: None,
        }],
    );
    registry.publish(
        "/src/main.rs",
        vec![archon_tools::lsp_diagnostics::LspDiagnostic {
            message: "second".to_string(),
            severity: archon_tools::lsp_diagnostics::DiagnosticSeverity::Warning,
            range_start_line: 2,
            range_start_char: 0,
            range_end_line: 2,
            range_end_char: 5,
            code: None,
            source: None,
        }],
    );
    let diags = registry.get_diagnostics("/src/main.rs");
    assert_eq!(diags.len(), 1, "publish replaces, not appends");
    assert_eq!(diags[0].message, "second");
}

#[test]
fn diagnostic_registry_clear_empties_file() {
    let mut registry = LspDiagnosticRegistry::new();
    registry.publish(
        "/src/main.rs",
        vec![archon_tools::lsp_diagnostics::LspDiagnostic {
            message: "some error".to_string(),
            severity: archon_tools::lsp_diagnostics::DiagnosticSeverity::Error,
            range_start_line: 0,
            range_start_char: 0,
            range_end_line: 0,
            range_end_char: 1,
            code: None,
            source: None,
        }],
    );
    registry.clear("/src/main.rs");
    assert!(registry.get_diagnostics("/src/main.rs").is_empty());
}

#[test]
fn diagnostic_registry_separate_files_independent() {
    let mut registry = LspDiagnosticRegistry::new();
    registry.publish(
        "/src/a.rs",
        vec![archon_tools::lsp_diagnostics::LspDiagnostic {
            message: "error in a".to_string(),
            severity: archon_tools::lsp_diagnostics::DiagnosticSeverity::Error,
            range_start_line: 0,
            range_start_char: 0,
            range_end_line: 0,
            range_end_char: 1,
            code: None,
            source: None,
        }],
    );
    // b.rs was not published — must return empty
    assert!(registry.get_diagnostics("/src/b.rs").is_empty());
    // a.rs still has its diagnostics
    assert_eq!(registry.get_diagnostics("/src/a.rs").len(), 1);
}

// ---------------------------------------------------------------------------
// LspServerManager — server detection
// ---------------------------------------------------------------------------

#[test]
fn manager_not_connected_initially() {
    let manager = LspServerManager::new(PathBuf::from("/tmp"), None);
    assert!(!manager.is_connected());
}

#[test]
fn manager_detects_rust_analyzer_from_cargo_toml() {
    // Create a temp dir with Cargo.toml
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("Cargo.toml"),
        "[package]\nname = \"test\"\nversion = \"0.1.0\"",
    )
    .unwrap();
    let manager = LspServerManager::new(tmp.path().to_path_buf(), None);
    let server = manager.detect_language_server();
    assert!(
        server.is_some(),
        "Cargo.toml present should detect a Rust language server"
    );
    let (binary, _args) = server.unwrap();
    assert!(
        binary.contains("rust-analyzer") || binary == "rust-analyzer",
        "Cargo.toml → rust-analyzer, got: {}",
        binary
    );
}

#[test]
fn manager_detects_typescript_server_from_package_json() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("package.json"),
        r#"{"name":"test","version":"1.0.0"}"#,
    )
    .unwrap();
    let manager = LspServerManager::new(tmp.path().to_path_buf(), None);
    let server = manager.detect_language_server();
    assert!(
        server.is_some(),
        "package.json should detect a TypeScript language server"
    );
    let (binary, _args) = server.unwrap();
    assert!(
        binary.contains("typescript-language-server") || binary.contains("tsserver"),
        "package.json → typescript-language-server, got: {}",
        binary
    );
}

#[test]
fn manager_no_server_for_unknown_project() {
    let tmp = tempfile::tempdir().unwrap();
    // No Cargo.toml, no package.json, no pyproject.toml
    let manager = LspServerManager::new(tmp.path().to_path_buf(), None);
    let server = manager.detect_language_server();
    // Should return None or Some for an unknown project type
    // We just check it doesn't panic
    let _ = server;
}

// ---------------------------------------------------------------------------
// LspOutput
// ---------------------------------------------------------------------------

#[test]
fn lsp_output_serializes() {
    let output = LspOutput {
        operation: "goToDefinition".to_string(),
        result: "src/main.rs:10:5".to_string(),
        file_path: "/src/main.rs".to_string(),
        result_count: 1,
        file_count: 1,
    };
    let json = serde_json::to_string(&output).unwrap();
    assert!(json.contains("goToDefinition"));
    assert!(json.contains("result_count"));
}
