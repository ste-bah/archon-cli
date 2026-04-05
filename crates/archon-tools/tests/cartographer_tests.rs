use archon_tools::cartographer::index::{CodebaseIndex, Symbol, SymbolKind};
use archon_tools::cartographer::parser::{language_for_file, parse_file};
use archon_tools::cartographer::summary::generate_summary;
use archon_tools::cartographer::CartographerTool;
use archon_tools::tool::Tool;


// ---------------------------------------------------------------------------
// Language detection
// ---------------------------------------------------------------------------

#[test]
fn language_detected_for_rs() {
    assert_eq!(language_for_file("foo.rs"), Some("rust"));
}

#[test]
fn language_detected_for_py() {
    assert_eq!(language_for_file("foo.py"), Some("python"));
}

#[test]
fn language_detected_for_ts() {
    assert_eq!(language_for_file("foo.ts"), Some("typescript"));
}

#[test]
fn language_detected_for_tsx() {
    assert_eq!(language_for_file("foo.tsx"), Some("typescript"));
}

#[test]
fn language_detected_for_go() {
    assert_eq!(language_for_file("foo.go"), Some("go"));
}

#[test]
fn unknown_extension_returns_none() {
    assert_eq!(language_for_file("foo.bin"), None);
}

#[test]
fn no_extension_returns_none() {
    assert_eq!(language_for_file("Makefile"), None);
}

// ---------------------------------------------------------------------------
// Rust parsing
// ---------------------------------------------------------------------------

#[test]
fn rust_struct_extracted() {
    let src = "pub struct MyStruct { pub field: u32 }";
    let syms = parse_file("test.rs", src, "rust");
    assert!(
        syms.iter().any(|s| s.name == "MyStruct" && s.kind == SymbolKind::Struct),
        "Expected MyStruct Struct, got: {syms:?}"
    );
}

#[test]
fn rust_function_extracted() {
    let src = "pub fn my_func(x: i32) -> bool { true }";
    let syms = parse_file("test.rs", src, "rust");
    assert!(
        syms.iter().any(|s| s.name == "my_func" && s.kind == SymbolKind::Function),
        "Expected my_func Function, got: {syms:?}"
    );
}

#[test]
fn rust_enum_extracted() {
    let src = "pub enum MyEnum { VariantA, VariantB }";
    let syms = parse_file("test.rs", src, "rust");
    assert!(
        syms.iter().any(|s| s.name == "MyEnum" && s.kind == SymbolKind::Enum),
        "Expected MyEnum Enum, got: {syms:?}"
    );
}

// ---------------------------------------------------------------------------
// Python parsing
// ---------------------------------------------------------------------------

#[test]
fn python_class_extracted() {
    let src = "class MyClass:\n    def method(self): pass";
    let syms = parse_file("test.py", src, "python");
    assert!(
        syms.iter().any(|s| s.name == "MyClass" && s.kind == SymbolKind::Class),
        "Expected MyClass Class, got: {syms:?}"
    );
}

#[test]
fn python_function_extracted() {
    let src = "def my_function(x, y):\n    return x + y";
    let syms = parse_file("test.py", src, "python");
    assert!(
        syms.iter().any(|s| s.name == "my_function" && s.kind == SymbolKind::Function),
        "Expected my_function Function, got: {syms:?}"
    );
}

// ---------------------------------------------------------------------------
// TypeScript parsing
// ---------------------------------------------------------------------------

#[test]
fn typescript_class_extracted() {
    let src = "class MyComponent { render() { return null; } }";
    let syms = parse_file("test.ts", src, "typescript");
    assert!(
        syms.iter().any(|s| s.name == "MyComponent" && s.kind == SymbolKind::Class),
        "Expected MyComponent Class, got: {syms:?}"
    );
}

#[test]
fn typescript_function_extracted() {
    let src = "function myFunc(a: string): void { console.log(a); }";
    let syms = parse_file("test.ts", src, "typescript");
    assert!(
        syms.iter().any(|s| s.name == "myFunc" && s.kind == SymbolKind::Function),
        "Expected myFunc Function, got: {syms:?}"
    );
}

// ---------------------------------------------------------------------------
// Go parsing
// ---------------------------------------------------------------------------

#[test]
fn go_function_extracted() {
    let src = "package main\nfunc MyGoFunc(x int) int { return x }";
    let syms = parse_file("test.go", src, "go");
    assert!(
        syms.iter().any(|s| s.name == "MyGoFunc" && s.kind == SymbolKind::Function),
        "Expected MyGoFunc Function, got: {syms:?}"
    );
}

// ---------------------------------------------------------------------------
// Index operations
// ---------------------------------------------------------------------------

#[test]
fn find_symbol_returns_results() {
    let mut index = CodebaseIndex::new();
    let sym = Symbol {
        name: "TargetSymbol".to_string(),
        kind: SymbolKind::Function,
        file: "src/main.rs".to_string(),
        line: 10,
        signature: "fn TargetSymbol()".to_string(),
    };
    index.symbols.insert("src/main.rs".to_string(), vec![sym]);

    let results = index.find_symbol("TargetSymbol");
    assert!(!results.is_empty(), "Expected to find TargetSymbol");
    assert_eq!(results[0].name, "TargetSymbol");
}

#[test]
fn find_symbol_partial_match() {
    let mut index = CodebaseIndex::new();
    let sym = Symbol {
        name: "calculate_discount".to_string(),
        kind: SymbolKind::Function,
        file: "src/lib.rs".to_string(),
        line: 5,
        signature: "fn calculate_discount(x: f64) -> f64".to_string(),
    };
    index.symbols.insert("src/lib.rs".to_string(), vec![sym]);

    let results = index.find_symbol("calculate");
    assert!(!results.is_empty(), "Partial name match should work");
}

#[test]
fn symbols_in_file_returns_slice() {
    let mut index = CodebaseIndex::new();
    let syms = vec![
        Symbol {
            name: "FuncA".to_string(),
            kind: SymbolKind::Function,
            file: "src/foo.rs".to_string(),
            line: 1,
            signature: "fn FuncA()".to_string(),
        },
        Symbol {
            name: "FuncB".to_string(),
            kind: SymbolKind::Function,
            file: "src/foo.rs".to_string(),
            line: 5,
            signature: "fn FuncB()".to_string(),
        },
    ];
    index.symbols.insert("src/foo.rs".to_string(), syms);

    let result = index.symbols_in_file("src/foo.rs");
    assert_eq!(result.len(), 2);
}

#[test]
fn symbols_in_file_missing_returns_empty() {
    let index = CodebaseIndex::new();
    let result = index.symbols_in_file("nonexistent.rs");
    assert!(result.is_empty());
}

// ---------------------------------------------------------------------------
// Summary
// ---------------------------------------------------------------------------

#[test]
fn summary_respects_token_budget() {
    let mut index = CodebaseIndex::new();
    let mut syms = Vec::new();
    for i in 0..100 {
        syms.push(Symbol {
            name: format!("function_{i}"),
            kind: SymbolKind::Function,
            file: "src/big.rs".to_string(),
            line: i,
            signature: format!("fn function_{i}(x: i32) -> i32"),
        });
    }
    index.symbols.insert("src/big.rs".to_string(), syms);

    let summary = generate_summary(&index, 10);
    // max_tokens=10 => max_chars=40, plus some tolerance for header
    assert!(
        summary.len() <= 200,
        "Summary should be truncated, got {} chars",
        summary.len()
    );
}

#[test]
fn summary_includes_symbol_names() {
    let mut index = CodebaseIndex::new();
    let sym = Symbol {
        name: "UniqueSymbolName".to_string(),
        kind: SymbolKind::Function,
        file: "src/lib.rs".to_string(),
        line: 1,
        signature: "fn UniqueSymbolName()".to_string(),
    };
    index.symbols.insert("src/lib.rs".to_string(), vec![sym]);

    let summary = generate_summary(&index, 1000);
    assert!(
        summary.contains("UniqueSymbolName"),
        "Summary should include symbol name"
    );
}

#[test]
fn summary_empty_index_returns_empty() {
    let index = CodebaseIndex::new();
    let summary = generate_summary(&index, 1000);
    assert!(summary.is_empty() || summary.trim().is_empty());
}

// ---------------------------------------------------------------------------
// Tool schema
// ---------------------------------------------------------------------------

#[test]
fn cartographer_tool_schema_is_object() {
    let schema = CartographerTool.input_schema();
    assert_eq!(
        schema["type"].as_str(),
        Some("object"),
        "Schema type must be object"
    );
}

#[test]
fn cartographer_tool_name_is_correct() {
    assert_eq!(CartographerTool.name(), "CartographerScan");
}

// ---------------------------------------------------------------------------
// Robustness: malformed source should not panic
// ---------------------------------------------------------------------------

#[test]
fn malformed_rust_source_returns_empty_or_partial() {
    let syms = parse_file("bad.rs", "this is not valid rust!!!@#$", "rust");
    // Should not panic — may return empty or some symbols, both OK
    let _ = syms.len();
}

#[test]
fn malformed_python_source_does_not_panic() {
    let syms = parse_file("bad.py", "def def class >>><<<!!!", "python");
    let _ = syms.len();
}

#[test]
fn empty_source_returns_empty() {
    let syms = parse_file("empty.rs", "", "rust");
    assert!(syms.is_empty(), "Empty source should yield no symbols");
}

#[test]
fn unknown_language_returns_empty() {
    let syms = parse_file("file.unknown", "some content", "cobol");
    assert!(syms.is_empty(), "Unknown language should yield no symbols");
}
