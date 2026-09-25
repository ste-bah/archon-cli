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

use std::ops::Range;

use super::tree_names::{declared_name, function_name, is_declaration, is_function};
use super::tree_regions::{misread_class, scan_c_definition, scan_macro};
use super::tree_roles::{Role, role};
use super::tree_score::{branch_count, signature};
use super::{FunctionScore, normalized_header};

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

/// Parse `text`, resolving a header: C++ when C++-only words appear in its
/// code (`class`, `namespace`, `template`, `typename`, `::`), else C unless
/// C++ reads it with less text in error nodes.
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

/// Judged on code only: a comment saying "this class of buffers" must not
/// flip a C header to C++.
fn looks_like_cpp(text: &str) -> bool {
    let syntax = super::source_text::Syntax::CFamily {
        preprocessor: true,
        raw_backticks: false,
    };
    super::source_text::code_lines(text, syntax)
        .iter()
        .any(|line| {
            line.code.contains("::")
                || line
                    .code
                    .split(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
                    .any(|word| matches!(word, "class" | "namespace" | "template" | "typename"))
        })
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
    /// Syntax errors outside every function that absorbs them: (1-based
    /// line, key of the innermost container holding it, `None` at file
    /// level).
    pub(super) stray_errors: Vec<(usize, Option<String>)>,
    /// Regions read by the hand scanner instead, and its unreliable
    /// readings there: (line, reason).
    pub(super) notes: Vec<(usize, String)>,
    /// The function-like nodes absorbed into another function.
    pub(super) nodes: Vec<FunctionScore>,
}

/// The context a node is visited in: whether a callback enclosing it
/// absorbs the callbacks in it, and the innermost enclosing container (an
/// index into [`Walk::containers`]).
type Context = (bool, Option<usize>);

/// `None` when the grammar cannot be loaded or the parse is abandoned; the
/// caller then falls back to the hand scanner.
pub(super) fn tree_scan(grammar: Grammar, text: &str) -> Option<TreeScan> {
    let (grammar, tree) = parse(grammar, text)?;
    let mut walk = Walk {
        grammar,
        text,
        out: TreeScan {
            grammar: Some(grammar),
            ..TreeScan::default()
        },
        containers: Vec::new(),
        scanned: Vec::new(),
    };
    let mut stack: Vec<(Node, Context)> = vec![(tree.root_node(), (false, None))];
    while let Some((node, context)) = stack.pop() {
        let Some(inner) = walk.visit(node, context) else {
            continue;
        };
        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();
        stack.extend(children.into_iter().rev().map(|child| (child, inner)));
    }
    let mut out = walk.out;
    out.stray_errors.dedup();
    out.functions.sort_by_key(|function| function.line);
    Some(out)
}

struct Walk<'text> {
    grammar: Grammar,
    text: &'text str,
    out: TreeScan,
    /// (enclosing container, key) per container met.
    containers: Vec<(Option<usize>, String)>,
    /// Byte ranges the hand scanner already read.
    scanned: Vec<Range<usize>>,
}

impl Walk<'_> {
    /// Handle one node; the context its children are visited in, or `None`
    /// when they are not visited.
    fn visit(&mut self, node: Node, (absorbing, container): Context) -> Option<Context> {
        if self
            .scanned
            .iter()
            .any(|range| range.contains(&node.start_byte()))
            || self.region(node)
        {
            return None;
        }
        if is_function(self.grammar, node) {
            return self.function(node, absorbing, container);
        }
        if (node.is_error() || node.is_missing()) && !absorbing {
            // Inside an absorbing callback the error marks that callback.
            let key = container.map(|index| self.containers[index].1.clone());
            self.out
                .stray_errors
                .push((node.start_position().row + 1, key));
        }
        Some((absorbing, container))
    }

    fn function(
        &mut self,
        node: Node,
        absorbing: bool,
        container: Option<usize>,
    ) -> Option<Context> {
        let role = role(self.grammar, node, self.text);
        let regions = chain(&self.containers, container);
        if absorbing && role == Role::Callback {
            // Absorbed by the enclosing callback, so not judged; kept as a
            // node for matching should an edit make it judged. What it
            // holds may still be judged.
            let node = function_at(self.grammar, node, self.text, role, regions);
            self.out.nodes.push(node);
            return Some((true, container));
        }
        let function = function_at(self.grammar, node, self.text, role, regions);
        // Numbered among identical containers (every `beforeEach(() => {`),
        // so an error in one excuses nothing in another; a number, not a
        // line, so moving code above does not change it.
        let same = format!("{}\u{0}{}", function.name, function.header);
        let seen = self
            .containers
            .iter()
            .filter(|(_, key)| key.starts_with(&format!("{same}\u{0}")))
            .count();
        let key = format!("{same}\u{0}{seen}");
        self.out.functions.push(function);
        match role {
            Role::Declaration => None,
            Role::Container => {
                self.containers.push((container, key));
                Some((false, Some(self.containers.len() - 1)))
            }
            Role::Callback => Some((true, container)),
        }
    }

    /// Hand the node to the hand scanner if the tree cannot read it; whether
    /// it did.
    fn region(&mut self, node: Node) -> bool {
        let c_family = matches!(self.grammar, Grammar::C | Grammar::Cpp | Grammar::CHeader);
        if c_family && node.kind() == "function_definition" {
            let class = misread_class(node, self.text);
            let read = class || declared_name(node).is_none();
            if read {
                scan_c_definition(node, self.text, class, &mut self.out, &mut self.scanned);
            }
            return read;
        }
        let macro_text = matches!(node.kind(), "macro_invocation" | "macro_definition");
        if self.grammar == Grammar::Rust && macro_text {
            scan_macro(node, self.text, &mut self.out);
        }
        self.grammar == Grammar::Rust && macro_text
    }
}

/// The keys of the containers enclosing `container`, innermost first.
fn chain(containers: &[(Option<usize>, String)], mut container: Option<usize>) -> Vec<String> {
    let mut keys = Vec::new();
    while let Some(index) = container {
        keys.push(containers[index].1.clone());
        container = containers[index].0;
    }
    keys
}

fn function_at(
    grammar: Grammar,
    node: Node,
    text: &str,
    role: Role,
    regions: Vec<String>,
) -> FunctionScore {
    let (name, line) = function_name(node, text);
    let body = node
        .child_by_field_name("body")
        .map_or(node.end_byte(), |body| body.start_byte());
    // What is judged on its own is left out of this function's score.
    let judged_apart = |nested: Node| match role {
        Role::Declaration => false,
        Role::Container => is_function(grammar, nested),
        Role::Callback => {
            is_function(grammar, nested)
                && super::tree_roles::role(grammar, nested, text) != Role::Callback
        }
    };
    // As an absorber: every nested callback in, named functions out.
    let named_apart = |nested: Node| is_function(grammar, nested) && is_declaration(nested);
    FunctionScore {
        name,
        line,
        score: 1 + branch_count(grammar, node, text, &judged_apart),
        header: normalized_header(&signature(node, body, text)),
        reliable: !node.has_error(),
        regions,
        end_line: node.end_position().row + 1,
        absorbed: 1 + branch_count(grammar, node, text, &named_apart),
        container: role == Role::Container,
    }
}
