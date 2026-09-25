//! Function spans and scores read from a syntax tree, for the languages with
//! a tree-sitter grammar in the workspace.
//!
//! The hand scanner (`brace_scan`, `source_text`) guesses where a function
//! starts and ends from its text, and every guess it gets wrong either
//! refuses a patch for a function that is not there or measures the wrong
//! span. A parser does not guess. For these languages the gate reads:
//!
//! - **functions**: Rust `fn` items (free, `impl` and trait default methods);
//!   Python `def` / `async def`; Go functions and receiver methods; Java
//!   methods and constructors; TypeScript / JavaScript function
//!   declarations, class and object methods, and arrow or `function`
//!   expressions assigned to a name (`const f = () => {}`, a class field).
//!   A function inside another counts toward the outer one, as closures and
//!   nested functions always have; an anonymous callback that is not inside
//!   a named function is not itself a function.
//! - **name and line**: the declared name and the line it is on.
//! - **score**: 1 plus one per keyword token in `BRANCH_TOKENS` and per
//!   `&&` / `||` operator anywhere in the function's tree, signature
//!   included — the hand scanner's token rule, applied to syntax tokens
//!   instead of words, so the cap means the same thing. Measured over this
//!   repository's 42k functions both readers find, 95% score identically and
//!   23 change side of a cap of 15. Differences: a Rust zero-argument
//!   closure's `||` is not an operator; an identifier spelled like a keyword
//!   (a Python variable `match`) does not count; strings, comments and
//!   docstrings never count; `&&` inside a template literal's `${...}` does.
//! - **signature**: the declaration text up to the body, comments dropped,
//!   for the ratchet's grouping.
//!
//! The parser recovers from syntax errors. A function whose own tree holds an
//! error node is marked unreliable (its span or score may be wrong); an
//! error outside every function is reported, because a function inside it
//! may not have been recognised.

use tree_sitter::{Language, Node, Parser};

use super::{BRANCH_TOKENS, FunctionScore, LOGICAL_OPERATORS, normalized_header};

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
        }
    }
}

/// What the tree says about one file.
#[derive(Debug, Default)]
pub(super) struct TreeScan {
    /// Every function, in file order; `reliable` is false where the
    /// function's own tree holds a syntax error.
    pub(super) functions: Vec<FunctionScore>,
    /// 1-based lines of syntax errors outside every function.
    pub(super) stray_errors: Vec<usize>,
}

/// `None` when the grammar cannot be loaded or the parse is abandoned; the
/// caller then falls back to the hand scanner.
pub(super) fn tree_scan(grammar: Grammar, text: &str) -> Option<TreeScan> {
    let mut parser = Parser::new();
    parser.set_language(&grammar.language()).ok()?;
    let tree = parser.parse(text, None)?;
    let mut out = TreeScan::default();
    let mut stack = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        if let Some(function) = function_at(grammar, node, text) {
            out.functions.push(function);
            continue;
        }
        if node.is_error() || node.is_missing() {
            out.stray_errors.push(node.start_position().row + 1);
        }
        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();
        stack.extend(children.into_iter().rev());
    }
    out.stray_errors.dedup();
    Some(out)
}

/// The function `node` declares, if it declares one.
fn function_at(grammar: Grammar, node: Node, text: &str) -> Option<FunctionScore> {
    let script = matches!(grammar, Grammar::TypeScript | Grammar::Tsx);
    let declared = matches!(
        (grammar, node.kind()),
        (Grammar::Rust, "function_item")
            | (Grammar::Python, "function_definition")
            | (Grammar::Go, "function_declaration" | "method_declaration")
            | (
                Grammar::Java,
                "method_declaration"
                    | "constructor_declaration"
                    | "compact_constructor_declaration"
            )
    ) || (script
        && matches!(
            node.kind(),
            "function_declaration" | "generator_function_declaration" | "method_definition"
        ));
    let (name, body) = if declared {
        (
            node.child_by_field_name("name")?,
            node.child_by_field_name("body")?,
        )
    } else if script
        && matches!(
            node.kind(),
            "variable_declarator" | "public_field_definition"
        )
    {
        let value = node.child_by_field_name("value")?;
        if !matches!(
            value.kind(),
            "arrow_function" | "function_expression" | "generator_function"
        ) {
            return None;
        }
        (
            node.child_by_field_name("name")?,
            value.child_by_field_name("body")?,
        )
    } else {
        return None;
    };
    Some(FunctionScore {
        name: text[name.byte_range()].to_string(),
        line: name.start_position().row + 1,
        score: 1 + branch_count(node, text),
        header: normalized_header(&signature(node, body.start_byte(), text)),
        reliable: !node.has_error(),
    })
}

/// Branch keywords and logical operators among the syntax tokens (unnamed
/// leaves) under `node`. String, comment and docstring contents are named
/// leaves, so they are never read.
fn branch_count(node: Node, text: &str) -> u32 {
    let mut count = 0usize;
    for leaf in leaves(node) {
        if leaf.is_named() || leaf.is_missing() {
            continue;
        }
        if BRANCH_TOKENS.contains(&leaf.kind()) {
            count += 1;
        }
        let token = &text[leaf.byte_range()];
        count += LOGICAL_OPERATORS
            .iter()
            .map(|operator| token.matches(operator).count())
            .sum::<usize>();
    }
    count as u32
}

/// The text of `node`'s tokens before byte `end` (its body), comments left
/// out, literal contents kept.
fn signature(node: Node, end: usize, text: &str) -> String {
    leaves(node)
        .into_iter()
        .filter(|leaf| leaf.end_byte() <= end)
        .map(|leaf| &text[leaf.byte_range()])
        .collect::<Vec<_>>()
        .join(" ")
}

/// Every leaf under `node` outside comments, in source order.
fn leaves(node: Node) -> Vec<Node> {
    let mut out = Vec::new();
    let mut stack = vec![node];
    while let Some(current) = stack.pop() {
        if current.kind().contains("comment") {
            continue;
        }
        if current.child_count() == 0 {
            out.push(current);
            continue;
        }
        let mut cursor = current.walk();
        let children: Vec<Node> = current.children(&mut cursor).collect();
        stack.extend(children.into_iter().rev());
    }
    out
}
