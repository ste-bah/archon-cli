//! Which syntax nodes are functions, and what to call each one.
//!
//! Every function-like node is judged — declarations, methods, and every
//! function expression, arrow function, `func` literal, lambda, closure and
//! Ruby block — unless it sits inside another judged function, where it
//! counts toward that one. Skipping unnamed forms let `module.exports = function () {}`,
//! `app.get('/', (req, res) => {})` and Go's `var H = func() {}` carry any
//! complexity unmeasured.
//!
//! Which of them absorbs which is decided by role (`tree_roles`):
//! declarations — and function values bound to a name, which are
//! declarations in all but syntax (`const f = () => {}`, a class field, an
//! object key, `module.exports = function () {}`, `export default`, Go's
//! `var H = func() {}`) — absorb everything nested in them; other callbacks
//! absorb the callbacks nested in them; only containers (test blocks,
//! callbacks holding two or more callbacks, file-root wrappers) leave their
//! nested functions to be judged apart.
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
        let mut declared = clean(text, name);
        if is_conversion(name)
            && let Some((before, _)) = declared.split_once('(')
        {
            declared = before.trim_end().to_string();
        }
        return (declared, line(name));
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
        // `operator bool()`, `Foo::operator int()`: a conversion operator.
        if is_conversion(current) {
            return Some(current);
        }
        current = current
            .child_by_field_name("declarator")
            .or_else(|| current.named_child(0))?;
    }
    None
}

/// An `operator_cast`, or a qualified name ending in one.
fn is_conversion(node: Node) -> bool {
    node.kind() == "operator_cast"
        || (node.kind() == "qualified_identifier"
            && node
                .child_by_field_name("name")
                .is_some_and(|name| name.kind() == "operator_cast"))
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
    let callee = call_callee(call, text).filter(|callee| !callee.is_empty());
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

/// What a call calls, by identifiers only (see [`callee_path`]).
pub(super) fn call_callee(call: Node, text: &str) -> Option<String> {
    if let Some(function) = call.child_by_field_name("function") {
        return Some(callee_path(function, text));
    }
    // Java `obj.m(...)`, Ruby `recv.m ...`: the call is its own member access.
    let member = call
        .child_by_field_name("name")
        .or_else(|| call.child_by_field_name("method"))?;
    let object = call
        .child_by_field_name("object")
        .or_else(|| call.child_by_field_name("receiver"));
    Some(joined(
        object.map(|object| root_name(object, text)),
        &clean(text, member),
    ))
}

/// A callee named by identifiers only: its root identifier and its final
/// member, with arguments, literals and intermediate calls dropped —
/// `rows.forEach`, `app.get`, `it.each` for `it.each([...])`, `fetch.then`
/// for `fetch(u).then(...).then`, `forEach` for `[[1, 2]].forEach`. The name
/// then survives a test table or a promise chain growing.
fn callee_path(node: Node, text: &str) -> String {
    if is_call(node) {
        return call_callee(node, text).unwrap_or_default();
    }
    if let Some((object, member)) = member_parts(node) {
        return joined(Some(root_name(object, text)), &clean(text, member));
    }
    identifier_text(node, text)
}

/// The identifier a member chain or call chain starts from, or empty.
fn root_name(node: Node, text: &str) -> String {
    if is_call(node) {
        let inner = node
            .child_by_field_name("function")
            .or_else(|| node.child_by_field_name("object"))
            .or_else(|| node.child_by_field_name("receiver"));
        return match inner {
            Some(inner) => root_name(inner, text),
            None => node
                .child_by_field_name("method")
                .or_else(|| node.child_by_field_name("name"))
                .map(|name| identifier_text(name, text))
                .unwrap_or_default(),
        };
    }
    match member_parts(node) {
        Some((object, _)) => root_name(object, text),
        None => identifier_text(node, text),
    }
}

fn is_call(node: Node) -> bool {
    matches!(
        node.kind(),
        "call_expression" | "call" | "method_invocation"
    )
}

/// (object, member) of a member access in any of the grammars.
fn member_parts(node: Node) -> Option<(Node, Node)> {
    const PAIRS: [(&str, &str); 6] = [
        ("object", "property"),
        ("value", "field"),
        ("operand", "field"),
        ("object", "field"),
        ("object", "attribute"),
        ("argument", "field"),
    ];
    PAIRS.iter().find_map(|(object, member)| {
        Some((
            node.child_by_field_name(object)?,
            node.child_by_field_name(member)?,
        ))
    })
}

/// An identifier-like node's text (path identifiers keep their `::`), else
/// empty: literals never become part of a name.
fn identifier_text(node: Node, text: &str) -> String {
    let kind = node.kind();
    let named = kind.contains("identifier")
        || matches!(
            kind,
            "constant" | "this" | "self" | "super" | "scope_resolution"
        );
    if named {
        text[node.byte_range()].split_whitespace().collect()
    } else {
        String::new()
    }
}

fn joined(root: Option<String>, member: &str) -> String {
    match root.filter(|root| !root.is_empty()) {
        Some(root) => cut(&format!("{root}.{member}")),
        None => cut(member),
    }
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
