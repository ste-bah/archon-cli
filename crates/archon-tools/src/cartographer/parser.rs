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
        "java" => Some("java"),
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
        "java" => extract_java_symbol(node, source, kind_str, file, line),
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

/// Java declaration node kinds that introduce a named type.
///
/// Used both to classify a node and to reconstruct the enclosing-type chain of
/// a nested declaration, so `Inner` is recorded as `Outer.Inner`.
const JAVA_TYPE_KINDS: &[&str] = &[
    "class_declaration",
    "interface_declaration",
    "enum_declaration",
    "record_declaration",
    "annotation_type_declaration",
];

fn extract_java_symbol(
    node: Node,
    source: &str,
    kind_str: &str,
    file: &str,
    line: usize,
) -> Option<Symbol> {
    let sym_kind = match kind_str {
        "class_declaration" => SymbolKind::Class,
        "interface_declaration" => SymbolKind::Interface,
        "enum_declaration" => SymbolKind::Enum,
        "record_declaration" => SymbolKind::Record,
        "annotation_type_declaration" => SymbolKind::Annotation,
        "method_declaration" | "constructor_declaration" => SymbolKind::Method,
        _ => return None,
    };

    let own_name = node
        .child_by_field_name("name")
        .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        .map(|s| s.to_string())?;

    // Java allows arbitrarily nested types and repeats method names freely
    // across classes, so a bare `Inner` or `process` does not identify anything.
    // Qualifying by the enclosing-type chain is what makes a lookup answerable.
    let mut name = java_enclosing_types(node, source);
    name.push(own_name);
    let name = name.join(".");

    Some(Symbol {
        name,
        kind: sym_kind,
        file: file.to_string(),
        line,
        signature: java_signature(node, source, 120),
    })
}

/// Names of the type declarations enclosing `node`, outermost first.
fn java_enclosing_types(node: Node, source: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut current = node.parent();
    while let Some(ancestor) = current {
        if JAVA_TYPE_KINDS.contains(&ancestor.kind())
            && let Some(name) = ancestor
                .child_by_field_name("name")
                .and_then(|n| n.utf8_text(source.as_bytes()).ok())
        {
            names.push(name.to_string());
        }
        current = ancestor.parent();
    }
    names.reverse();
    names
}

/// Build a Java signature from the declaration's own text, excluding its body.
///
/// The generic first-line rule does not work here: tree-sitter puts annotations
/// inside the declaration node, so an `@Override`-annotated method would record
/// `@Override` as its signature. Declarations also wrap across lines far more
/// often than in the other indexed languages, so the text is collapsed onto one
/// line rather than truncated at the first newline.
fn java_signature(node: Node, source: &str, max_chars: usize) -> String {
    let start = java_signature_start(node);
    let end = node
        .child_by_field_name("body")
        .map(|body| body.start_byte())
        .unwrap_or_else(|| node.end_byte())
        .max(start);

    source
        .get(start..end)
        .unwrap_or("")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max_chars)
        .collect()
}

/// Byte offset of the first non-annotation token of a Java declaration.
///
/// Modifiers and annotations share one `modifiers` node, so the annotations are
/// skipped individually to keep `public`/`static`/`final` in the signature.
fn java_signature_start(node: Node) -> usize {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() != "modifiers" {
            return child.start_byte();
        }
        let mut modifier_cursor = child.walk();
        for modifier in child.children(&mut modifier_cursor) {
            if !matches!(modifier.kind(), "marker_annotation" | "annotation") {
                return modifier.start_byte();
            }
        }
    }
    node.start_byte()
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
        "java" => Some(tree_sitter_java::LANGUAGE.into()),
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
