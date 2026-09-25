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

use super::tree_names::{function_name, is_function};
use super::{
    BRANCH_TOKENS, FunctionScore, LOGICAL_OPERATORS, RUBY_BRANCH_TOKENS, hand_scan,
    normalized_header,
};

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
            "rb" => Self::Ruby,
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

    /// Branch keywords beyond `BRANCH_TOKENS` this language has. Ruby's
    /// `elsif`, `unless`, `until`, `when` and `rescue` are its spellings of
    /// `elif`, `if`, `while`, `case` and `catch`, and it counts its
    /// `and` / `or` operators with `&&` / `||`.
    fn extra_branch_tokens(self) -> &'static [&'static str] {
        match self {
            Self::Ruby => RUBY_BRANCH_TOKENS,
            _ => &[],
        }
    }
}

/// Parse `text`, resolving a header to the grammar that reads it cleanly.
fn parse(grammar: Grammar, text: &str) -> Option<(Grammar, Tree)> {
    let tree = parse_with(grammar, text)?;
    if grammar != Grammar::CHeader {
        return Some((grammar, tree));
    }
    if !tree.root_node().has_error() {
        return Some((Grammar::C, tree));
    }
    match parse_with(Grammar::Cpp, text) {
        Some(cpp) if !cpp.root_node().has_error() => Some((Grammar::Cpp, cpp)),
        _ => Some((Grammar::C, tree)),
    }
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
    /// Unreliable hand-scanner readings of macro bodies: (line, reason).
    pub(super) macro_notes: Vec<(usize, String)>,
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
            out.functions.push(function_at(grammar, node, text));
            continue;
        }
        if grammar == Grammar::Rust
            && matches!(node.kind(), "macro_invocation" | "macro_definition")
        {
            scan_macro(node, text, &mut out);
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
    out.functions.sort_by_key(|function| function.line);
    Some(out)
}

fn function_at(grammar: Grammar, node: Node, text: &str) -> FunctionScore {
    let (name, line) = function_name(node, text);
    let body = node
        .child_by_field_name("body")
        .map_or(node.end_byte(), |body| body.start_byte());
    FunctionScore {
        name,
        line,
        score: 1 + branch_count(grammar, node, text),
        header: normalized_header(&signature(node, body, text)),
        reliable: !node.has_error(),
    }
}

/// The hand scanner's reading of a top-level macro's text, its lines moved
/// to where the macro sits in the file.
fn scan_macro(node: Node, text: &str, out: &mut TreeScan) {
    let offset = node.start_position().row;
    let scan = hand_scan("macro.rs", &text[node.byte_range()]);
    out.functions
        .extend(scan.functions.into_iter().map(|mut function| {
            function.line += offset;
            function
        }));
    out.macro_notes.extend(
        scan.unreliable
            .into_iter()
            .map(|(line, reason)| (line + offset, format!("inside a macro: {reason}"))),
    );
}

/// Branch keywords and logical operators among the syntax tokens (unnamed
/// leaves) under `node`. String, comment and docstring contents are named
/// leaves, so they are never read.
fn branch_count(grammar: Grammar, node: Node, text: &str) -> u32 {
    let mut count = 0usize;
    let extra = grammar.extra_branch_tokens();
    for leaf in leaves(node) {
        if leaf.is_named() || leaf.is_missing() {
            continue;
        }
        if BRANCH_TOKENS.contains(&leaf.kind()) || extra.contains(&leaf.kind()) {
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
