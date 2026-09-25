//! Which syntax nodes are functions, and what to call each one.
//!
//! Every function-like node is judged — declarations, methods, and every
//! function expression, arrow function, `func` literal, lambda, closure and
//! Ruby block — unless it sits inside another judged function, where it
//! counts toward that one. Skipping unnamed forms let `module.exports = function () {}`,
//! `app.get('/', (req, res) => {})` and Go's `var H = func() {}` carry any
//! complexity unmeasured.
//!
//! Declarations — and function values bound to a name, which are
//! declarations in all but syntax (`const f = () => {}`, a class field, an
//! object key, `module.exports = function () {}`, `export default`, Go's
//! `var H = func() {}`) — absorb every closure and nested function inside
//! them. An anonymous callback, IIFE or block outside every declaration is a
//! *container*: it scores only its own tokens and each function inside it
//! is judged on its own, under the same rule. Otherwise a test suite, an
//! RSpec `describe` block or a UMD wrapper added its every case to one
//! score, and adding a case "grew" it. (A `.map` callback inside a container
//! is split out too: one rule, rather than guessing which calls register
//! code.)
//!
//! A function without a declared name is named from its context: the
//! binding above, `default` for a default export, or the call it is passed
//! to — its identifiers only, arguments dropped, plus a first string or
//! symbol argument (`it('adds')`, `scope(:active)`, `setTimeout(...)`).
//! Failing all of those it is `<anonymous>`. The name never carries a line
//! number or a data argument, so an unchanged function keeps its name — and
//! its ratchet pairing — when code above it moves or a test table grows.
//!
//! A C / C++ `function_definition` without a declarator name is never a
//! function: an export or framework macro made the parser read a class or
//! namespace as one.

use tree_sitter::Node;

use super::tree_scan::Grammar;

/// Longest name kept; a longer context (a long assignment target) is cut.
const MAX_NAME: usize = 80;

/// Whether `node` is a function the gate judges.
pub(super) fn is_function(grammar: Grammar, node: Node) -> bool {
    let kind = node.kind();
    let function_like = match grammar {
        Grammar::Rust => matches!(kind, "function_item" | "closure_expression"),
        Grammar::Python => matches!(kind, "function_definition" | "lambda"),
        Grammar::Go => matches!(
            kind,
            "function_declaration" | "method_declaration" | "func_literal"
        ),
        Grammar::Java => matches!(
            kind,
            "method_declaration"
                | "constructor_declaration"
                | "compact_constructor_declaration"
                | "lambda_expression"
        ),
        Grammar::TypeScript | Grammar::Tsx => matches!(
            kind,
            "function_declaration"
                | "generator_function_declaration"
                | "function_expression"
                | "function"
                | "generator_function"
                | "arrow_function"
                | "method_definition"
        ),
        Grammar::C => kind == "function_definition",
        Grammar::Cpp => matches!(kind, "function_definition" | "lambda_expression"),
        // A header resolves to C or C++ before any node is read.
        Grammar::CHeader => kind == "function_definition",
        // A lambda's `{ }` / `do end` body is that lambda, not a second block.
        Grammar::Ruby => {
            matches!(kind, "method" | "singleton_method" | "lambda")
                || (matches!(kind, "block" | "do_block")
                    && node.parent().is_none_or(|parent| parent.kind() != "lambda"))
        }
    };
    // A bodiless declaration (abstract or interface method) has nothing to
    // measure.
    let c_family = matches!(grammar, Grammar::C | Grammar::Cpp | Grammar::CHeader);
    function_like
        && node.child_by_field_name("body").is_some()
        && !(c_family && kind == "function_definition" && declared_name(node).is_none())
}

/// Whether a function node absorbs the functions nested in it: a declaration,
/// or a function value bound to a name.
pub(super) fn is_declaration(node: Node) -> bool {
    let declared = matches!(
        node.kind(),
        "function_item"
            | "function_definition"
            | "function_declaration"
            | "method_declaration"
            | "constructor_declaration"
            | "compact_constructor_declaration"
            | "generator_function_declaration"
            | "method_definition"
            | "method"
            | "singleton_method"
    );
    declared || binding(node).is_some()
}

/// The node a function value is bound to a name through, if any.
fn binding(node: Node) -> Option<Node> {
    let mut parent = node.parent()?;
    // Go binds `var H = func() {}` through an expression list.
    if parent.kind() == "expression_list" {
        parent = parent.parent()?;
    }
    matches!(
        parent.kind(),
        "variable_declarator"
            | "public_field_definition"
            | "field_definition"
            | "var_spec"
            | "const_spec"
            | "static_item"
            | "const_item"
            | "pair"
            | "assignment_expression"
            | "assignment"
            | "assignment_statement"
            | "short_var_declaration"
            | "augmented_assignment"
            | "let_declaration"
            | "init_declarator"
            | "export_statement"
    )
    .then_some(parent)
}

/// The function's name and the 1-based line it is reported at.
pub(super) fn function_name(node: Node, text: &str) -> (String, usize) {
    if let Some(name) = node.child_by_field_name("name") {
        // Ruby `def self.x` / `def obj.x`.
        let object = node
            .child_by_field_name("object")
            .map(|object| format!("{}.", clean(text, object)))
            .unwrap_or_default();
        return (cut(&format!("{object}{}", clean(text, name))), line(name));
    }
    // C / C++: the name is inside the declarator (`A::b`, `*f`, `operator+`).
    if node.kind() == "function_definition"
        && let Some(name) = declared_name(node)
    {
        return (clean(text, name), line(name));
    }
    let mut parent = node.parent();
    // Go binds `var H = func() {}` through an expression list.
    if parent.is_some_and(|parent| parent.kind() == "expression_list") {
        parent = parent.and_then(|parent| parent.parent());
    }
    let Some(parent) = parent else {
        return anonymous(node);
    };
    let bound = match parent.kind() {
        "variable_declarator"
        | "public_field_definition"
        | "field_definition"
        | "var_spec"
        | "const_spec"
        | "static_item"
        | "const_item"
        | "keyword_argument" => parent.child_by_field_name("name"),
        "pair" => parent.child_by_field_name("key"),
        "assignment_expression"
        | "assignment"
        | "assignment_statement"
        | "short_var_declaration"
        | "augmented_assignment" => parent.child_by_field_name("left"),
        "let_declaration" => parent.child_by_field_name("pattern"),
        "init_declarator" => parent.child_by_field_name("declarator"),
        // A Ruby block belongs to the call it is attached to.
        "call" if parent.child_by_field_name("block") == Some(node) => {
            return call_name(parent, node, text);
        }
        "export_statement" => return ("default".to_string(), line(node)),
        "arguments" | "argument_list" => return callee_name(parent, node, text),
        _ => None,
    };
    match bound {
        Some(name) => (clean(text, name), line(name)),
        None => anonymous(node),
    }
}

/// The name inside a C / C++ declarator chain (`int *f(void)`, `A::b()`).
pub(super) fn declared_name(node: Node) -> Option<Node> {
    let mut current = node.child_by_field_name("declarator")?;
    for _ in 0..8 {
        if current.kind() == "function_declarator" {
            return current.child_by_field_name("declarator");
        }
        current = current
            .child_by_field_name("declarator")
            .or_else(|| current.named_child(0))?;
    }
    None
}

/// A function passed as an argument, named after the call.
fn callee_name(arguments: Node, node: Node, text: &str) -> (String, usize) {
    match arguments.parent() {
        Some(call) => call_name(call, node, text),
        None => anonymous(node),
    }
}

/// `callee(<first string or symbol argument>)` or `callee(...)`.
fn call_name(call: Node, node: Node, text: &str) -> (String, usize) {
    let callee = call
        .child_by_field_name("function")
        .or_else(|| call.child_by_field_name("name"))
        .map(|callee| callee_path(callee, text))
        .or_else(|| ruby_callee(call, text))
        .filter(|callee| !callee.is_empty());
    let Some(callee) = callee else {
        return anonymous(node);
    };
    let label = call
        .child_by_field_name("arguments")
        .and_then(|arguments| arguments.named_child(0))
        .filter(|first| first.id() != node.id() && is_label(*first))
        .map(|first| clean(text, first))
        .unwrap_or_else(|| "...".to_string());
    (cut(&format!("{callee}({label})")), line(node))
}

/// A first argument stable enough to name a callback by: a string or symbol
/// literal, never data such as a test table.
fn is_label(node: Node) -> bool {
    let kind = node.kind();
    kind.contains("string") || matches!(kind, "simple_symbol" | "delimited_symbol")
}

/// Ruby's `receiver.method` for a call.
fn ruby_callee(call: Node, text: &str) -> Option<String> {
    let method = callee_path(call.child_by_field_name("method")?, text);
    Some(match call.child_by_field_name("receiver") {
        Some(receiver) => format!("{}.{method}", callee_path(receiver, text)),
        None => method,
    })
}

/// A callee's identifiers and separators with every argument list left out:
/// `it.each([[1, 2]])` is `it.each`, however its table grows.
fn callee_path(node: Node, text: &str) -> String {
    let mut out = String::new();
    let mut stack = vec![node];
    while let Some(current) = stack.pop() {
        let kind = current.kind();
        if matches!(
            kind,
            "arguments" | "argument_list" | "argument_list_with_parens"
        ) || kind.contains("string")
            || kind.contains("comment")
        {
            continue;
        }
        if current.child_count() == 0 {
            out.push_str(&text[current.byte_range()]);
            continue;
        }
        let mut cursor = current.walk();
        let children: Vec<Node> = current.children(&mut cursor).collect();
        stack.extend(children.into_iter().rev());
    }
    cut(&out)
}

fn anonymous(node: Node) -> (String, usize) {
    ("<anonymous>".to_string(), line(node))
}

fn line(node: Node) -> usize {
    node.start_position().row + 1
}

/// The node's text with whitespace runs collapsed, cut to [`MAX_NAME`].
fn clean(text: &str, node: Node) -> String {
    let words: Vec<&str> = text[node.byte_range()].split_whitespace().collect();
    cut(&words.join(" "))
}

fn cut(name: &str) -> String {
    name.chars().take(MAX_NAME).collect()
}
