//! Which syntax nodes are functions, and what to call each one.
//!
//! Every function-like node is judged — declarations, methods, and every
//! function expression, arrow function, `func` literal, lambda, closure and
//! Ruby block — unless it sits inside another judged function, where it
//! counts toward that one. Skipping unnamed forms let `module.exports = function () {}`,
//! `app.get('/', (req, res) => {})` and Go's `var H = func() {}` carry any
//! complexity unmeasured.
//!
//! A function without a declared name is named from its context: the
//! variable, field, object key or assignment target it is bound to, `default`
//! for a default export, or the call it is passed to (`app.get('/')`,
//! `setTimeout(...)`). Failing all of those it is `<anonymous>`. The name
//! never carries a line number, so an unchanged function keeps its name — and
//! its ratchet pairing — when code above it moves.

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
        Grammar::Ruby => matches!(
            kind,
            "method" | "singleton_method" | "block" | "do_block" | "lambda"
        ),
    };
    // A bodiless declaration (abstract or interface method) has nothing to
    // measure.
    function_like && node.child_by_field_name("body").is_some()
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
fn declared_name(node: Node) -> Option<Node> {
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

/// `callee(<first string argument>)` or `callee(...)`.
fn call_name(call: Node, node: Node, text: &str) -> (String, usize) {
    let callee = call
        .child_by_field_name("function")
        .or_else(|| call.child_by_field_name("name"))
        .map(|callee| clean(text, callee))
        .or_else(|| ruby_callee(call, text));
    let Some(callee) = callee else {
        return anonymous(node);
    };
    let label = call
        .child_by_field_name("arguments")
        .and_then(|arguments| arguments.named_child(0))
        .filter(|first| first.id() != node.id() && first.kind().contains("string"))
        .map(|first| clean(text, first))
        .unwrap_or_else(|| "...".to_string());
    (cut(&format!("{callee}({label})")), line(node))
}

/// Ruby's `receiver.method` for a call.
fn ruby_callee(call: Node, text: &str) -> Option<String> {
    let method = clean(text, call.child_by_field_name("method")?);
    Some(match call.child_by_field_name("receiver") {
        Some(receiver) => format!("{}.{method}", clean(text, receiver)),
        None => method,
    })
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
