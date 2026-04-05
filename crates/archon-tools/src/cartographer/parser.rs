use tree_sitter::Node;

use super::index::{Symbol, SymbolKind};

/// Determine the language identifier for a file path based on extension.
pub fn language_for_file(path: &str) -> Option<&'static str> {
    match std::path::Path::new(path).extension()?.to_str()? {
        "rs" => Some("rust"),
        "py" => Some("python"),
        "ts" | "tsx" => Some("typescript"),
        "js" | "jsx" => Some("javascript"),
        "go" => Some("go"),
        _ => None,
    }
}

/// Parse `source` as `language` and return all extracted symbols.
///
/// Returns an empty `Vec` on any parse failure — never panics.
pub fn parse_file(path: &str, source: &str, language: &str) -> Vec<Symbol> {
    if source.is_empty() {
        return Vec::new();
    }

    let ts_language = match get_ts_language(language) {
        Some(l) => l,
        None => {
            tracing::warn!(
                "No tree-sitter grammar for language '{}' (file: {})",
                language,
                path
            );
            return Vec::new();
        }
    };

    let mut parser = tree_sitter::Parser::new();
    if let Err(e) = parser.set_language(&ts_language) {
        tracing::warn!("Failed to set tree-sitter language '{}': {}", language, e);
        return Vec::new();
    }

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => {
            tracing::warn!("tree-sitter failed to parse file: {}", path);
            return Vec::new();
        }
    };

    let root = tree.root_node();
    let mut symbols = Vec::new();
    extract_symbols(root, source, language, path, &mut symbols);
    symbols
}

/// Recursively walk the AST and extract symbol declarations.
fn extract_symbols(node: Node, source: &str, language: &str, file: &str, out: &mut Vec<Symbol>) {
    walk_tree(node, &mut |n| {
        if let Some(sym) = extract_node_symbol(n, source, language, file) {
            out.push(sym);
        }
    });
}

/// Attempt to extract a `Symbol` from a single AST node.
fn extract_node_symbol(node: Node, source: &str, language: &str, file: &str) -> Option<Symbol> {
    let kind_str = node.kind();
    let line = node.start_position().row + 1;

    match language {
        "rust" => extract_rust_symbol(node, source, kind_str, file, line),
        "python" => extract_python_symbol(node, source, kind_str, file, line),
        "typescript" | "javascript" => extract_ts_symbol(node, source, kind_str, file, line),
        "go" => extract_go_symbol(node, source, kind_str, file, line),
        _ => None,
    }
}

fn extract_rust_symbol(
    node: Node,
    source: &str,
    kind_str: &str,
    file: &str,
    line: usize,
) -> Option<Symbol> {
    let (sym_kind, name_field) = match kind_str {
        "struct_item" => (SymbolKind::Struct, "name"),
        "function_item" => (SymbolKind::Function, "name"),
        "enum_item" => (SymbolKind::Enum, "name"),
        "trait_item" => (SymbolKind::Interface, "name"),
        "type_item" => (SymbolKind::Type, "name"),
        _ => return None,
    };

    let name = node
        .child_by_field_name(name_field)
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        .map(|s| s.to_string())?;

    let signature = extract_signature(node, source, 120);

    Some(Symbol {
        name,
        kind: sym_kind,
        file: file.to_string(),
        line,
        signature,
    })
}

fn extract_python_symbol(
    node: Node,
    source: &str,
    kind_str: &str,
    file: &str,
    line: usize,
) -> Option<Symbol> {
    let sym_kind = match kind_str {
        "class_definition" => SymbolKind::Class,
        "function_definition" => SymbolKind::Function,
        _ => return None,
    };

    let name = node
        .child_by_field_name("name")
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        .map(|s| s.to_string())?;

    let signature = extract_signature(node, source, 120);

    Some(Symbol {
        name,
        kind: sym_kind,
        file: file.to_string(),
        line,
        signature,
    })
}

fn extract_ts_symbol(
    node: Node,
    source: &str,
    kind_str: &str,
    file: &str,
    line: usize,
) -> Option<Symbol> {
    let sym_kind = match kind_str {
        "class_declaration" => SymbolKind::Class,
        "function_declaration" => SymbolKind::Function,
        "interface_declaration" => SymbolKind::Interface,
        "type_alias_declaration" => SymbolKind::Type,
        "method_definition" => SymbolKind::Method,
        _ => return None,
    };

    let name = node
        .child_by_field_name("name")
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        .map(|s| s.to_string())?;

    let signature = extract_signature(node, source, 120);

    Some(Symbol {
        name,
        kind: sym_kind,
        file: file.to_string(),
        line,
        signature,
    })
}

fn extract_go_symbol(
    node: Node,
    source: &str,
    kind_str: &str,
    file: &str,
    line: usize,
) -> Option<Symbol> {
    let sym_kind = match kind_str {
        "function_declaration" => SymbolKind::Function,
        "method_declaration" => SymbolKind::Method,
        "type_declaration" => SymbolKind::Type,
        _ => return None,
    };

    let name = node
        .child_by_field_name("name")
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        .map(|s| s.to_string())?;

    let signature = extract_signature(node, source, 120);

    Some(Symbol {
        name,
        kind: sym_kind,
        file: file.to_string(),
        line,
        signature,
    })
}

/// Get the first line of a node's text as its signature, up to `max_chars`.
fn extract_signature(node: Node, source: &str, max_chars: usize) -> String {
    node.utf8_text(source.as_bytes())
        .unwrap_or("")
        .lines()
        .next()
        .unwrap_or("")
        .chars()
        .take(max_chars)
        .collect()
}

/// Get a tree-sitter `Language` for the given language identifier.
fn get_ts_language(language: &str) -> Option<tree_sitter::Language> {
    match language {
        "rust" => Some(tree_sitter_rust::LANGUAGE.into()),
        "python" => Some(tree_sitter_python::LANGUAGE.into()),
        "typescript" => Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        "javascript" => Some(tree_sitter_typescript::LANGUAGE_TSX.into()),
        "go" => Some(tree_sitter_go::LANGUAGE.into()),
        _ => None,
    }
}

/// Recursively walk tree, calling `f` for each node.
fn walk_tree<F: FnMut(Node)>(node: Node, f: &mut F) {
    f(node);
    let count = node.child_count();
    for i in 0..count {
        if let Some(child) = node.child(i) {
            walk_tree(child, f);
        }
    }
}
