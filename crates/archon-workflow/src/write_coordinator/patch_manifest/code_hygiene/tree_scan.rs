//! Function spans and scores read from a syntax tree, for the languages with
//! a tree-sitter grammar in the workspace.
//!
//! The hand scanner (`brace_scan`, `source_text`) guesses where a function
//! starts and ends from its text, and every guess it gets wrong either
//! refuses a patch for a function that is not there or measures the wrong
//! span. A parser does not guess. For these languages the gate reads:
//!
//! - **functions**: every function-like node not inside another judged
//!   function, named from its context (`tree_names`):
//!   Rust `function_item`, `closure_expression`; Python
//!   `function_definition`, `lambda`; Go `function_declaration`,
//!   `method_declaration`, `func_literal`; Java `method_declaration`,
//!   `constructor_declaration`, `lambda_expression`; TypeScript/JavaScript
//!   function declarations and expressions, generators, `arrow_function`,
//!   `method_definition`; C `function_definition`; C++
//!   `function_definition` (class-body methods, out-of-line `A::b`,
//!   templates) and `lambda_expression`; Ruby `method`, `singleton_method`,
//!   `block`, `do_block`, `lambda`. Rust functions inside a macro invocation
//!   or `macro_rules!` body are flat token trees to the parser, so each
//!   top-level macro's text is read by the hand scanner. A `.h` header is
//!   read as C, or as C++ when only C++ parses it without errors.
//! - **name and line**: the declared or contextual name and its line.
//! - **score**: 1 plus one per keyword token in `BRANCH_TOKENS` (`if` —
//!   including `else if` and modifier `if` — `for`, `while` — including
//!   `do ... while` — `match`, `case` labels, `catch`, `elif`, `except`)
//!   and per `&&` / `||` operator anywhere in the function's tree,
//!   signature included; Ruby also counts `RUBY_BRANCH_TOKENS` (`elsif`,
//!   `unless`, `until`, `when`, `rescue`, `and`, `or`). This is the hand
//!   scanner's token rule applied to syntax tokens instead of words, so the
//!   cap means the same thing: `switch`, `do`, `?:` and C++'s `and` / `or`
//!   stay uncounted as before, and preprocessor `#if` is not `if`. Measured
//!   over this repository's 42k functions both readers find, 95% score
//!   identically and 23 change side of a cap of 15. Differences: a Rust
//!   zero-argument closure's `||` is not an operator; an identifier spelled
//!   like a keyword (a Python variable `match`) does not count; strings,
//!   comments and docstrings never count; `&&` inside a template literal's
//!   `${...}` does.
//! - **signature**: the declaration text up to the body, comments dropped,
//!   for the ratchet's grouping.
//!
//! The parser recovers from syntax errors. A function whose own tree holds an
//! error node is still judged on its recovered score, but marked as holding
//! one; an error outside every function is reported, because a function
//! inside it may not have been recognised.

use tree_sitter::{Language, Node, Parser, Tree};

use super::tree_names::{declared_name, function_name, is_declaration, is_function};
use super::tree_score::{branch_count, signature};
use super::{FunctionScore, hand_scan, normalized_header};

/// A grammar the complexity gate parses with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Grammar {
    Rust,
    Python,
    TypeScript,
    /// TypeScript with JSX; also reads JavaScript.
    Tsx,
    Go,
    Java,
    C,
    Cpp,
    /// A `.h` header: read as C, or as C++ when only C++ parses it cleanly.
    CHeader,
    Ruby,
}

impl Grammar {
    pub(super) fn for_path(path: &str) -> Option<Self> {
        let ext = path.rsplit_once('.').map(|(_, ext)| ext)?;
        Some(match ext {
            "rs" => Self::Rust,
            "py" | "pyi" => Self::Python,
            "ts" => Self::TypeScript,
            "tsx" | "js" | "jsx" | "mjs" => Self::Tsx,
            "go" => Self::Go,
            "java" => Self::Java,
            "c" => Self::C,
            "h" => Self::CHeader,
            "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => Self::Cpp,
            "rb" | "rake" => Self::Ruby,
            "cjs" => Self::Tsx,
            "mts" | "cts" => Self::TypeScript,
            "inl" | "ipp" | "tpp" => Self::Cpp,
            _ => return None,
        })
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Python => "python",
            Self::TypeScript => "typescript",
            Self::Tsx => "tsx",
            Self::Go => "go",
            Self::Java => "java",
            Self::C | Self::CHeader => "c",
            Self::Cpp => "cpp",
            Self::Ruby => "ruby",
        }
    }

    fn language(self) -> Language {
        match self {
            Self::Rust => tree_sitter_rust::LANGUAGE.into(),
            Self::Python => tree_sitter_python::LANGUAGE.into(),
            Self::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Self::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Self::Go => tree_sitter_go::LANGUAGE.into(),
            Self::Java => tree_sitter_java::LANGUAGE.into(),
            Self::C | Self::CHeader => tree_sitter_c::LANGUAGE.into(),
            Self::Cpp => tree_sitter_cpp::LANGUAGE.into(),
            Self::Ruby => tree_sitter_ruby::LANGUAGE.into(),
        }
    }
}

/// Parse `text`, resolving a header: C++ when C++-only words appear
/// (`class`, `namespace`, `template`, `typename`, `::`), else C unless C++
/// reads it with less text in error nodes.
fn parse(grammar: Grammar, text: &str) -> Option<(Grammar, Tree)> {
    if grammar != Grammar::CHeader {
        return parse_with(grammar, text).map(|tree| (grammar, tree));
    }
    if looks_like_cpp(text)
        && let Some(cpp) = parse_with(Grammar::Cpp, text)
    {
        return Some((Grammar::Cpp, cpp));
    }
    let c = parse_with(Grammar::C, text)?;
    if !c.root_node().has_error() {
        return Some((Grammar::C, c));
    }
    match parse_with(Grammar::Cpp, text) {
        Some(cpp) if error_weight(&cpp) < error_weight(&c) => Some((Grammar::Cpp, cpp)),
        _ => Some((Grammar::C, c)),
    }
}

fn looks_like_cpp(text: &str) -> bool {
    text.contains("::")
        || text
            .split(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
            .any(|word| matches!(word, "class" | "namespace" | "template" | "typename"))
}

/// Bytes covered by error nodes, plus one per missing node.
fn error_weight(tree: &Tree) -> usize {
    let mut weight = 0;
    let mut stack = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        if node.is_error() {
            weight += node.byte_range().len();
            continue;
        }
        weight += usize::from(node.is_missing());
        if node.has_error() {
            let mut cursor = node.walk();
            stack.extend(node.children(&mut cursor));
        }
    }
    weight
}

fn parse_with(grammar: Grammar, text: &str) -> Option<Tree> {
    let mut parser = Parser::new();
    parser.set_language(&grammar.language()).ok()?;
    parser.parse(text, None)
}

/// What the tree says about one file.
#[derive(Debug, Default)]
pub(super) struct TreeScan {
    /// The grammar that read the file (a header resolved to C or C++).
    pub(super) grammar: Option<Grammar>,
    /// Every function, in file order; `reliable` is false where the
    /// function's own tree holds a syntax error.
    pub(super) functions: Vec<FunctionScore>,
    /// 1-based lines of syntax errors outside every function.
    pub(super) stray_errors: Vec<usize>,
    /// Regions read by the hand scanner instead, and its unreliable
    /// readings there: (line, reason).
    pub(super) notes: Vec<(usize, String)>,
}

/// `None` when the grammar cannot be loaded or the parse is abandoned; the
/// caller then falls back to the hand scanner.
pub(super) fn tree_scan(grammar: Grammar, text: &str) -> Option<TreeScan> {
    let (grammar, tree) = parse(grammar, text)?;
    let mut out = TreeScan {
        grammar: Some(grammar),
        ..TreeScan::default()
    };
    let mut stack = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        if is_function(grammar, node) {
            let declared = is_declaration(node);
            out.functions
                .push(function_at(grammar, node, text, !declared));
            if declared {
                continue;
            }
        } else if unnamed_c_function(grammar, node) {
            let reason = "the parser read this as a function without a name (often a macro); \
                          read by the hand scanner instead";
            out.notes
                .push((node.start_position().row + 1, reason.to_string()));
            scan_region(node, text, "region.cpp", &mut out);
            continue;
        } else if grammar == Grammar::Rust
            && matches!(node.kind(), "macro_invocation" | "macro_definition")
        {
            scan_region(node, text, "macro.rs", &mut out);
            continue;
        } else if node.is_error() || node.is_missing() {
            // Outside every declaration (a container's included): a
            // function may be hidden in it.
            out.stray_errors.push(node.start_position().row + 1);
        }
        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();
        stack.extend(children.into_iter().rev());
    }
    out.stray_errors.dedup();
    out.functions.sort_by_key(|function| function.line);
    Some(out)
}

/// A C / C++ `function_definition` with no declarator name: an export or
/// framework macro (`class API_EXPORT Foo`, `Q_OBJECT`) made the parser
/// read a class or namespace as one function. Never judged as one.
fn unnamed_c_function(grammar: Grammar, node: Node) -> bool {
    matches!(grammar, Grammar::C | Grammar::Cpp | Grammar::CHeader)
        && node.kind() == "function_definition"
        && declared_name(node).is_none()
}

/// `own_only` for a container: its nested functions are judged separately.
fn function_at(grammar: Grammar, node: Node, text: &str, own_only: bool) -> FunctionScore {
    let (name, line) = function_name(node, text);
    let body = node
        .child_by_field_name("body")
        .map_or(node.end_byte(), |body| body.start_byte());
    FunctionScore {
        name,
        line,
        score: 1 + branch_count(grammar, node, text, own_only),
        header: normalized_header(&signature(grammar, node, body, text)),
        reliable: !node.has_error(),
    }
}

/// The hand scanner's reading of a region the tree cannot read as code (a
/// macro's token tree, a macro-mangled C / C++ definition), its lines moved
/// to where the region sits in the file. `as_path` picks the syntax.
fn scan_region(node: Node, text: &str, as_path: &str, out: &mut TreeScan) {
    let body = &text[node.byte_range()];
    let (functions, notes) = region_functions(body, as_path, node.start_position().row);
    out.functions.extend(functions);
    out.notes.extend(notes);
}

/// A Rust macro body read by the hand scanner (see [`region_functions`]).
#[cfg(test)]
pub(super) fn macro_functions(
    body: &str,
    offset: usize,
) -> (Vec<FunctionScore>, Vec<(usize, String)>) {
    region_functions(body, "macro.rs", offset)
}

/// Functions the hand scanner finds in `body`, which starts on 0-based row
/// `offset`. A reading that lost sync is not judged at all — every span in
/// it is suspect — and is returned as a note instead.
fn region_functions(
    body: &str,
    as_path: &str,
    offset: usize,
) -> (Vec<FunctionScore>, Vec<(usize, String)>) {
    let scan = hand_scan(as_path, body);
    let notes = scan
        .unreliable
        .into_iter()
        .map(|(line, reason)| {
            (
                line + offset,
                format!("in a region the hand scanner read: {reason}"),
            )
        })
        .collect();
    if scan.lost_sync {
        return (Vec::new(), notes);
    }
    let functions = scan
        .functions
        .into_iter()
        .map(|mut function| {
            function.line += offset;
            function
        })
        .collect();
    (functions, notes)
}
